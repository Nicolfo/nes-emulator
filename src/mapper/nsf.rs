//! Synthetic "board" that plays an NSF music rip on the emulated console.
//!
//! The NSF's program data is mapped at $8000-$FFFF (4KB banks switched via
//! $5FF8-$5FFF, or loaded linearly at the load address), with 8KB of work RAM
//! at $6000-$7FFF. A small 6502 driver lives in cartridge register space at
//! $4100-$41FF, which no expansion chip decodes; the reset/NMI/IRQ vectors
//! are overridden to point into it. The driver clears RAM, initialises the
//! APU, calls the tune's init routine with the song number in A and the
//! region in X, then idles. A CPU-cycle timer on this board raises IRQ at the
//! header's play rate; the driver's IRQ handler acknowledges it and calls the
//! play routine. Running the real code on the real CPU/APU keeps the timing
//! (and the tune's sound) cycle-accurate.

use serde::{Deserialize, Serialize};

use super::fds_audio::FdsAudio;
use super::fme7::Sunsoft5b;
use super::mmc5::Mmc5Audio;
use super::n163::N163Audio;
use super::vrc6::Vrc6Audio;
use super::vrc7::Opll;
use super::{Mapper, Mirroring};
use crate::cartridge::Region;
use crate::nsf::{EXP_5B, EXP_FDS, EXP_MMC5, EXP_N163, EXP_VRC6, EXP_VRC7, NsfHeader, NsfInfo};

/// Base address of the driver program.
const DRIVER_BASE: u16 = 0x4100;
/// Driver register: current song number (read).
const REG_SONG: u16 = 0x41F0;
/// Driver register: region, 0 = NTSC, 1 = PAL (read).
const REG_REGION: u16 = 0x41F1;
/// Driver register: read acknowledges the play IRQ; bit 7 = it was pending.
const REG_ACK: u16 = 0x41F2;
/// Driver register: any write starts the play timer (after init returns).
const REG_START: u16 = 0x41F3;

const RESET_ENTRY: u16 = DRIVER_BASE;
const IRQ_ENTRY: u16 = DRIVER_BASE + 0x66;
const NMI_ENTRY: u16 = DRIVER_BASE + 0x78;
/// Offsets of the JSR operands patched with the init/play addresses.
const INIT_OPERAND: usize = 0x5D;
const PLAY_OPERAND: usize = 0x71;

/// Hand-assembled driver (offsets relative to $4100; see the module docs).
#[rustfmt::skip]
const DRIVER: [u8; 0x79] = [
    // $4100 reset: SEI / CLD / LDX #$FF / TXS
    0x78, 0xD8, 0xA2, 0xFF, 0x9A,
    // LDA #0 / STA $2000 / STA $2001 / TAX
    0xA9, 0x00, 0x8D, 0x00, 0x20, 0x8D, 0x01, 0x20, 0xAA,
    // $410E: clear $0000-$07FF (STA $0n00,X for n = 0..7), INX, BNE
    0x9D, 0x00, 0x00, 0x9D, 0x00, 0x01, 0x9D, 0x00, 0x02, 0x9D, 0x00, 0x03,
    0x9D, 0x00, 0x04, 0x9D, 0x00, 0x05, 0x9D, 0x00, 0x06, 0x9D, 0x00, 0x07,
    0xE8, 0xD0, 0xE5,
    // $4129: LDY #0 / STY $00 / LDX #$60 / STX $01
    0xA0, 0x00, 0x84, 0x00, 0xA2, 0x60, 0x86, 0x01,
    // $4131: STA ($00),Y / INY / BNE -5 / INC $01 / LDX $01 / CPX #$80 / BNE
    0x91, 0x00, 0xC8, 0xD0, 0xFB, 0xE6, 0x01, 0xA6, 0x01, 0xE0, 0x80, 0xD0, 0xF3,
    // $413E: STA $01 / TAX
    0x85, 0x01, 0xAA,
    // $4141: STA $4000,X / INX / CPX #$14 / BNE
    0x9D, 0x00, 0x40, 0xE8, 0xE0, 0x14, 0xD0, 0xF8,
    // $4149: STA $4015 ($00) / LDA #$0F / STA $4015 / LDA #$40 / STA $4017
    0x8D, 0x15, 0x40, 0xA9, 0x0F, 0x8D, 0x15, 0x40, 0xA9, 0x40, 0x8D, 0x17, 0x40,
    // $4156: LDA song / LDX region / JSR init
    0xAD, 0xF0, 0x41, 0xAE, 0xF1, 0x41, 0x20, 0x00, 0x00,
    // $415F: STA start / CLI
    0x8D, 0xF3, 0x41, 0x58,
    // $4163: idle: JMP $4163
    0x4C, 0x63, 0x41,
    // $4166 irq: PHA / TXA / PHA / TYA / PHA / LDA ack / BPL skip / JSR play
    0x48, 0x8A, 0x48, 0x98, 0x48, 0xAD, 0xF2, 0x41, 0x10, 0x03, 0x20, 0x00, 0x00,
    // $4173 skip: PLA / TAY / PLA / TAX / PLA
    0x68, 0xA8, 0x68, 0xAA, 0x68,
    // $4178 nmi (and the IRQ handler's tail): RTI
    0x40,
];

/// Expansion chips, each present only when the header enables it.
#[derive(Serialize, Deserialize, Default)]
struct Chips {
    vrc6: Option<Vrc6Audio>,
    vrc7: Option<Opll>,
    n163: Option<N163Audio>,
    s5b: Option<Sunsoft5b>,
    mmc5: Option<Mmc5Audio>,
    fds: Option<FdsAudio>,
}

impl Chips {
    fn new(expansion: u8) -> Self {
        Chips {
            vrc6: (expansion & EXP_VRC6 != 0).then(Vrc6Audio::new),
            vrc7: (expansion & EXP_VRC7 != 0).then(Opll::new),
            n163: (expansion & EXP_N163 != 0).then(N163Audio::new),
            s5b: (expansion & EXP_5B != 0).then(Sunsoft5b::new),
            mmc5: (expansion & EXP_MMC5 != 0).then(Mmc5Audio::new),
            fds: (expansion & EXP_FDS != 0).then(FdsAudio::new),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct NsfMapper {
    /// Program image, a whole number of 4KB banks.
    #[serde(skip)]
    prg: Vec<u8>,
    prg_ram: Vec<u8>,
    chr: Vec<u8>,
    /// MMC5 ExRAM ($5C00-$5FF5), plain RAM for MMC5 tunes.
    exram: Vec<u8>,
    /// FDS tunes run from RAM: $6000-$FFFF is 40KB of RAM loaded from the
    /// program image (banks copied in by $5FF6-$5FFF writes). Empty otherwise.
    fds_ram: Vec<u8>,
    banks: [u8; 8],
    bank_init: [u8; 8],
    bankswitched: bool,
    init_addr: u16,
    play_addr: u16,
    region: Region,
    title: String,
    artist: String,
    copyright: String,
    total_songs: usize,
    song: usize,
    expansion: u8,
    /// Play period in CPU cycles.
    period: u32,
    counter: u32,
    timer_running: bool,
    irq_line: bool,
    multiplicand: u8,
    multiplier: u8,
    /// Initial FDS banks for the $6000/$7000 slots (header $76/$77).
    fds_low_init: [u8; 2],
    chips: Chips,
}

/// FDS RAM spans $6000-$FFFF.
const FDS_RAM_SIZE: usize = 0xA000;

impl NsfMapper {
    pub fn new(h: &NsfHeader, data: &[u8]) -> Self {
        let fds = h.expansion & EXP_FDS != 0;
        let prg = if h.bankswitched {
            // Data starts at the load address's offset within its 4KB bank.
            let pad = (h.load_addr & 0x0FFF) as usize;
            let mut v = vec![0u8; pad];
            v.extend_from_slice(data);
            let len = v.len().max(0x1000).next_multiple_of(0x1000);
            v.resize(len, 0);
            v
        } else if fds {
            // Linear 40KB image of $6000-$FFFF (FDS tunes may load at $6000).
            let pad = h.load_addr.saturating_sub(0x6000) as usize;
            let mut v = vec![0u8; FDS_RAM_SIZE];
            let n = data.len().min(FDS_RAM_SIZE - pad);
            v[pad..pad + n].copy_from_slice(&data[..n]);
            v
        } else {
            // Linear 32KB image with the data placed at the load address.
            let pad = (h.load_addr - 0x8000) as usize;
            let mut v = vec![0u8; 0x8000];
            let n = data.len().min(0x8000 - pad);
            v[pad..pad + n].copy_from_slice(&data[..n]);
            v
        };
        let bank_init = if h.bankswitched {
            h.bank_init
        } else {
            [0, 1, 2, 3, 4, 5, 6, 7]
        };
        // FDS: $5FF6/$5FF7 (the $6000/$7000 slots) start at header $76/$77.
        let fds_low_init = [h.bank_init[6], h.bank_init[7]];
        let (cpu_hz, us) = match h.region {
            Region::Ntsc => (1_789_773.0, h.speed_ntsc_us),
            Region::Pal => (1_662_607.0, h.speed_pal_us),
        };
        let period = ((us as f64 * cpu_hz / 1_000_000.0).round() as u32).max(1);
        let mut m = NsfMapper {
            prg,
            prg_ram: vec![0; 0x2000],
            chr: vec![0; 0x2000],
            exram: vec![0; 0x400],
            fds_ram: if fds {
                vec![0; FDS_RAM_SIZE]
            } else {
                Vec::new()
            },
            fds_low_init,
            banks: bank_init,
            bank_init,
            bankswitched: h.bankswitched,
            init_addr: h.init_addr,
            play_addr: h.play_addr,
            region: h.region,
            title: h.title.clone(),
            artist: h.artist.clone(),
            copyright: h.copyright.clone(),
            total_songs: h.total_songs,
            song: h.start_song,
            expansion: h.expansion,
            period,
            counter: period,
            timer_running: false,
            irq_line: false,
            multiplicand: 0xFF,
            multiplier: 0xFF,
            chips: Chips::new(h.expansion),
        };
        m.load_fds_ram();
        m
    }

    /// (Re)load the FDS RAM image from the program banks. Runs at power-on
    /// and again when the driver reads the song number right before calling
    /// init - i.e. after its RAM-clear loop has zeroed $6000-$7FFF.
    fn load_fds_ram(&mut self) {
        if self.fds_ram.is_empty() {
            return;
        }
        if self.bankswitched {
            for slot in 0..FDS_RAM_SIZE / 0x1000 {
                let bank = if slot < 2 {
                    self.fds_low_init[slot]
                } else {
                    self.banks[slot - 2]
                };
                self.copy_bank_to_fds_slot(slot, bank);
            }
        } else {
            self.fds_ram.copy_from_slice(&self.prg);
        }
    }

    /// FDS bankswitching copies a 4KB program bank into RAM slot `slot`
    /// (0 = $6000 ... 9 = $F000).
    fn copy_bank_to_fds_slot(&mut self, slot: usize, bank: u8) {
        let nbanks = self.prg.len() / 0x1000;
        let src = (bank as usize % nbanks) * 0x1000;
        self.fds_ram[slot * 0x1000..(slot + 1) * 0x1000]
            .copy_from_slice(&self.prg[src..src + 0x1000]);
    }

    fn driver_byte(&self, addr: u16) -> u8 {
        let off = (addr - DRIVER_BASE) as usize;
        let [init_lo, init_hi] = self.init_addr.to_le_bytes();
        let [play_lo, play_hi] = self.play_addr.to_le_bytes();
        match off {
            INIT_OPERAND => init_lo,
            o if o == INIT_OPERAND + 1 => init_hi,
            PLAY_OPERAND => play_lo,
            o if o == PLAY_OPERAND + 1 => play_hi,
            o => DRIVER.get(o).copied().unwrap_or(0xEA),
        }
    }

    fn prg_byte(&self, addr: u16) -> u8 {
        let nbanks = self.prg.len() / 0x1000;
        let bank = self.banks[((addr - 0x8000) >> 12) as usize] as usize % nbanks;
        self.prg[bank * 0x1000 + (addr as usize & 0x0FFF)]
    }
}

impl Mapper for NsfMapper {
    crate::impl_mapper_savestate!(prg_ram, chr, exram, fds_ram);

    fn cpu_read(&mut self, addr: u16) -> u8 {
        let vector = |a: u16, hi: bool| if hi { (a >> 8) as u8 } else { a as u8 };
        match addr {
            0xFFFA | 0xFFFB => vector(NMI_ENTRY, addr & 1 != 0),
            0xFFFC | 0xFFFD => vector(RESET_ENTRY, addr & 1 != 0),
            0xFFFE | 0xFFFF => vector(IRQ_ENTRY, addr & 1 != 0),
            0x8000..=0xFFF9 if !self.fds_ram.is_empty() => self.fds_ram[addr as usize - 0x6000],
            0x8000..=0xFFF9 => self.prg_byte(addr),
            _ => 0,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if !self.fds_ram.is_empty() {
            match addr {
                // FDS RAM; expansion-chip registers below still see the write.
                0x6000..=0xDFFF => self.fds_ram[addr as usize - 0x6000] = val,
                0x5FF6..=0x5FFF if self.bankswitched => {
                    return self.copy_bank_to_fds_slot((addr - 0x5FF6) as usize, val);
                }
                _ => {}
            }
        }
        let chips = &mut self.chips;
        match addr {
            REG_START => {
                self.timer_running = true;
                self.counter = self.period;
            }
            0x5FF8..=0x5FFF if self.bankswitched => self.banks[(addr - 0x5FF8) as usize] = val,
            0x6000..=0x7FFF => self.prg_ram[(addr & 0x1FFF) as usize] = val,
            0x4040..=0x408A => {
                if let Some(f) = &mut chips.fds {
                    f.write(addr, val);
                }
            }
            0x4800 => {
                if let Some(n) = &mut chips.n163 {
                    n.data_write(val);
                }
            }
            0xF800..=0xFFFF if chips.n163.is_some() => {
                if let Some(n) = &mut chips.n163 {
                    n.addr_write(val);
                }
            }
            0x5000..=0x5015 => {
                if let Some(m) = &mut chips.mmc5 {
                    m.write(addr, val);
                }
            }
            0x5205 if chips.mmc5.is_some() => self.multiplicand = val,
            0x5206 if chips.mmc5.is_some() => self.multiplier = val,
            0x5C00..=0x5FF5 if chips.mmc5.is_some() => self.exram[(addr & 0x3FF) as usize] = val,
            0x9010 | 0x9030 => {
                if let Some(o) = &mut chips.vrc7 {
                    if addr == 0x9010 {
                        o.write_addr(val);
                    } else {
                        o.write_data(val);
                    }
                }
            }
            0x9000..=0x9003 | 0xA000..=0xA002 | 0xB000..=0xB002 => {
                if let Some(v) = &mut chips.vrc6 {
                    v.write(addr, val);
                }
            }
            0xC000..=0xDFFF => {
                if let Some(s) = &mut chips.s5b {
                    s.reg_select = val & 0x0F;
                }
            }
            0xE000..=0xFFFF => {
                if let Some(s) = &mut chips.s5b {
                    s.write_reg(val);
                }
            }
            _ => {}
        }
    }

    fn cpu_reg_read(&mut self, addr: u16) -> Option<u8> {
        match addr {
            REG_SONG => {
                // The driver reads this right before JSR init, after its
                // RAM clear: the moment to (re)load an FDS tune's RAM.
                self.load_fds_ram();
                Some(self.song as u8)
            }
            REG_REGION => Some((self.region == Region::Pal) as u8),
            REG_ACK => {
                let v = (self.irq_line as u8) << 7;
                self.irq_line = false;
                Some(v)
            }
            DRIVER_BASE..=0x41FF => Some(self.driver_byte(addr)),
            0x4040..=0x4092 => self.chips.fds.as_ref().and_then(|f| f.read(addr)),
            0x4800 => self.chips.n163.as_mut().map(|n| n.data_read()),
            0x5015 => self.chips.mmc5.as_ref().map(|m| m.status()),
            0x5205 | 0x5206 if self.chips.mmc5.is_some() => {
                let p = self.multiplicand as u16 * self.multiplier as u16;
                Some(if addr == 0x5205 {
                    p as u8
                } else {
                    (p >> 8) as u8
                })
            }
            0x5C00..=0x5FF5 if self.chips.mmc5.is_some() => {
                Some(self.exram[(addr & 0x3FF) as usize])
            }
            _ => None,
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.chr[addr as usize & 0x1FFF]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        self.chr[addr as usize & 0x1FFF] = val;
    }

    fn mirroring(&self) -> Mirroring {
        Mirroring::Horizontal
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        if !self.fds_ram.is_empty() {
            return Some(self.fds_ram[addr as usize - 0x6000]);
        }
        Some(self.prg_ram[(addr & 0x1FFF) as usize])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.prg_ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.prg_ram)
    }

    fn irq(&self) -> bool {
        self.irq_line
    }

    fn cpu_clock(&mut self) {
        if self.timer_running {
            self.counter -= 1;
            if self.counter == 0 {
                self.counter = self.period;
                self.irq_line = true;
            }
        }
        let c = &mut self.chips;
        if let Some(v) = &mut c.vrc6 {
            v.clock();
        }
        if let Some(o) = &mut c.vrc7 {
            o.clock();
        }
        if let Some(n) = &mut c.n163 {
            n.clock();
        }
        if let Some(s) = &mut c.s5b {
            s.clock();
        }
        if let Some(m) = &mut c.mmc5 {
            m.clock();
        }
        if let Some(f) = &mut c.fds {
            f.clock();
        }
    }

    fn audio_sample(&self) -> f32 {
        let c = &self.chips;
        c.vrc6.as_ref().map_or(0.0, |v| v.sample())
            + c.vrc7.as_ref().map_or(0.0, |o| o.output())
            + c.n163.as_ref().map_or(0.0, |n| n.sample())
            + c.s5b.as_ref().map_or(0.0, |s| s.sample())
            + c.mmc5.as_ref().map_or(0.0, |m| m.sample())
            + c.fds.as_ref().map_or(0.0, |f| f.sample())
    }

    fn nsf_info(&self) -> Option<NsfInfo> {
        Some(NsfInfo {
            title: self.title.clone(),
            artist: self.artist.clone(),
            copyright: self.copyright.clone(),
            total_songs: self.total_songs,
            current_song: self.song,
            expansion: self.expansion,
        })
    }

    fn nsf_select(&mut self, song: usize) {
        self.song = song.min(self.total_songs - 1);
        self.banks = self.bank_init;
        self.timer_running = false;
        self.counter = self.period;
        self.irq_line = false;
        // Fresh chips so the previous track's notes don't ring on.
        self.chips = Chips::new(self.expansion);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_branches_and_vectors_line_up() {
        // Spot-check the hand-assembled layout against the entry constants.
        assert_eq!(DRIVER[0x63], 0x4C, "idle JMP at $4163");
        assert_eq!(&DRIVER[0x64..0x66], &[0x63, 0x41]);
        assert_eq!(DRIVER[(IRQ_ENTRY - DRIVER_BASE) as usize], 0x48, "irq PHA");
        assert_eq!(DRIVER[(NMI_ENTRY - DRIVER_BASE) as usize], 0x40, "RTI");
        assert_eq!(DRIVER[INIT_OPERAND - 1], 0x20, "JSR init");
        assert_eq!(DRIVER[PLAY_OPERAND - 1], 0x20, "JSR play");
    }

    fn mapper(expansion: u8) -> NsfMapper {
        let mut data = vec![0u8; 0x80];
        data[0..5].copy_from_slice(b"NESM\x1A");
        data[6] = 1;
        data[7] = 1;
        data[0x08..0x0A].copy_from_slice(&0x8000u16.to_le_bytes());
        data[0x7B] = expansion;
        data.extend_from_slice(&[0x60; 16]);
        let (h, body) = crate::nsf::parse(&data).unwrap();
        NsfMapper::new(&h, body)
    }

    #[test]
    fn vectors_point_into_driver() {
        let mut m = mapper(0);
        let vec =
            |m: &mut NsfMapper, a: u16| m.cpu_read(a) as u16 | (m.cpu_read(a + 1) as u16) << 8;
        assert_eq!(vec(&mut m, 0xFFFC), RESET_ENTRY);
        assert_eq!(vec(&mut m, 0xFFFE), IRQ_ENTRY);
        assert_eq!(vec(&mut m, 0xFFFA), NMI_ENTRY);
        assert_eq!(m.cpu_reg_read(DRIVER_BASE), Some(0x78));
    }

    #[test]
    fn expansion_chips_only_when_enabled() {
        let mut plain = mapper(0);
        plain.cpu_write(0x9000, 0x8F);
        plain.cpu_write(0x9002, 0x80);
        plain.cpu_clock();
        assert_eq!(plain.audio_sample(), 0.0, "no VRC6 without the header bit");
        assert_eq!(plain.cpu_reg_read(0x5205), None);

        let mut m = mapper(EXP_VRC6 | EXP_MMC5 | EXP_N163 | EXP_5B);
        // VRC6 pulse 1: constant-output mode, volume 15, enabled.
        m.cpu_write(0x9000, 0x8F);
        m.cpu_write(0x9002, 0x80);
        m.cpu_clock();
        assert!(m.audio_sample() > 0.0, "VRC6 pulse audible");
        // MMC5 multiplier and ExRAM.
        m.cpu_write(0x5205, 12);
        m.cpu_write(0x5206, 34);
        assert_eq!(m.cpu_reg_read(0x5205), Some((12 * 34) as u8));
        m.cpu_write(0x5C10, 0x5A);
        assert_eq!(m.cpu_reg_read(0x5C10), Some(0x5A));
        // $F800 is the N163 address port, not a 5B data write.
        m.cpu_write(0xF800, 0x80 | 0x20);
        m.cpu_write(0x4800, 0x77);
        m.cpu_write(0xF800, 0x20);
        assert_eq!(m.cpu_reg_read(0x4800), Some(0x77));
    }

    /// FDS tune: load address, bank-init bytes, program body.
    fn fds_mapper(load: u16, banks: [u8; 8], body: &[u8]) -> NsfMapper {
        let mut data = vec![0u8; 0x80];
        data[0..5].copy_from_slice(b"NESM\x1A");
        data[6] = 1;
        data[7] = 1;
        data[0x08..0x0A].copy_from_slice(&load.to_le_bytes());
        data[0x70..0x78].copy_from_slice(&banks);
        data[0x7B] = EXP_FDS;
        data.extend_from_slice(body);
        let (h, body) = crate::nsf::parse(&data).unwrap();
        NsfMapper::new(&h, body)
    }

    #[test]
    fn fds_tune_runs_from_writable_ram_loaded_at_6000() {
        let mut m = fds_mapper(0x6000, [0; 8], &[0xAA, 0xBB]);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAA));
        assert_eq!(m.prg_ram_read(0x6001), Some(0xBB));
        // The driver's RAM clear wipes $6000; reading the song reloads it.
        m.cpu_write(0x6000, 0);
        m.cpu_write(0x9000, 0x42); // $8000-$DFFF is RAM too
        assert_eq!(m.cpu_read(0x9000), 0x42);
        m.cpu_reg_read(REG_SONG);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAA));
    }

    #[test]
    fn fds_bankswitch_copies_banks_into_ram() {
        let mut body = vec![0u8; 0x3000];
        body[0x0000] = 0x10;
        body[0x1000] = 0x11;
        body[0x2000] = 0x12;
        let mut m = fds_mapper(0x8000, [0, 1, 2, 0, 0, 0, 2, 1], &body);
        assert_eq!(m.cpu_read(0x8000), 0x10);
        assert_eq!(m.cpu_read(0x9000), 0x11);
        assert_eq!(m.prg_ram_read(0x6000), Some(0x12), "$5FF6 init from $76");
        assert_eq!(m.prg_ram_read(0x7000), Some(0x11), "$5FF7 init from $77");
        m.cpu_write(0x5FF8, 2);
        assert_eq!(m.cpu_read(0x8000), 0x12);
        m.cpu_write(0x5FF6, 0);
        assert_eq!(m.prg_ram_read(0x6000), Some(0x10));
    }

    #[test]
    fn fds_audio_registers_routed() {
        let mut m = fds_mapper(0x8000, [0; 8], &[0; 0x1000]);
        assert!(m.cpu_reg_read(0x4090).is_some(), "FDS status readable");
        assert_eq!(mapper(0).cpu_reg_read(0x4090), None);
    }
}
