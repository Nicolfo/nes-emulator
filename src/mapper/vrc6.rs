use super::{Mapper, Mirroring, NtTarget, mirror_nt};
use serde::{Deserialize, Serialize};

/// VRC6 (mapper 24, Akumajou Densetsu): 16KB+8KB PRG banking, the VRC
/// scanline/cycle IRQ, expansion audio (two pulse channels with variable
/// duty plus a sawtooth channel), and the full $B003 "PPU banking style"
/// register: four CHR layouts (1KB, 2KB, mixed), CIRAM or CHR-ROM
/// nametables with per-mode page selection, the CHR A10 rule and the PRG
/// RAM enable. Commercial games only use mode 0 with CIRAM ($20-$2C,
/// $A0-$AC), which behaves exactly like plain register-selected mirroring.
///
/// Mapper 26 is the same chip on the VRC6b pinout, where the A0 and A1 lines
/// feeding the register decoder are swapped; everything else is identical.
#[derive(Serialize, Deserialize)]
pub struct Vrc6 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    #[serde(default)]
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    mirroring: Mirroring,
    prg_16k: u8,
    prg_8k: u8,
    chr_banks: [u8; 8],
    /// $B003. `None` only for savestates written before the register was
    /// modelled: they behave as the mode-0 value matching `mirroring`.
    #[serde(default)]
    ppu_mode: Option<u8>,
    irq: VrcIrq,
    audio: Vrc6Audio,
    // VRC6b swaps the A0/A1 register-select lines relative to VRC6a.
    swap_lines: bool,
}

impl Vrc6 {
    /// VRC6a wiring (mapper 24): register lines map straight to A0/A1.
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        Self::with_lines(prg, chr, mirroring, false)
    }

    /// VRC6b wiring (mapper 26): A0 and A1 are swapped.
    pub fn new_vrc6b(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        Self::with_lines(prg, chr, mirroring, true)
    }

    fn with_lines(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring, swap_lines: bool) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Vrc6 {
            prg,
            chr,
            chr_is_ram,
            prg_ram: vec![0; 0x2000],
            mirroring,
            prg_16k: 0,
            prg_8k: 0,
            chr_banks: [0; 8],
            ppu_mode: Some(0),
            irq: VrcIrq::new(),
            audio: Vrc6Audio::new(),
            swap_lines,
        }
    }

    /// Effective $B003 value (see `ppu_mode`).
    fn mode(&self) -> u8 {
        self.ppu_mode.unwrap_or_else(|| {
            // Legacy savestate: the old core only modelled mode 0 mirroring.
            let mm = match self.mirroring {
                Mirroring::Horizontal => 1,
                Mirroring::SingleScreenLo => 2,
                Mirroring::SingleScreenHi => 3,
                _ => 0,
            };
            0xA0 | (mm << 2)
        })
    }

    /// 1KB CHR page for pattern slot `slot` (0-7). In 2KB layouts P=1
    /// (bit 5) passes PPU A10 through (even/odd page of the register);
    /// P=0 takes A10 from the register's LSB, duplicating one half.
    fn pattern_page(&self, slot: usize) -> u8 {
        let mode = self.mode();
        let r = &self.chr_banks;
        let two_k = |reg: u8| {
            if mode & 0x20 != 0 {
                (reg & 0xFE) | (slot as u8 & 1)
            } else {
                reg
            }
        };
        match mode & 3 {
            0 => r[slot],
            1 => two_k(r[slot / 2]),
            // Modes 2/3: $0000-$0FFF in 1KB pages, $1000-$1FFF in 2KB.
            _ if slot < 4 => r[slot],
            _ => two_k(r[4 + (slot - 4) / 2]),
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x400;
        let bank = self.pattern_page((addr >> 10) as usize & 7) as usize % banks;
        bank * 0x400 + (addr as usize & 0x3FF)
    }

    /// Page backing nametable `nt` (0-3): a CHR page when N (bit 4) is set,
    /// otherwise a CIRAM page (only its bit 0 matters).
    fn nametable_page(&self, nt: usize) -> u8 {
        let mode = self.mode();
        let r = &self.chr_banks;
        let (r6, r7) = (r[6], r[7]);
        let rom = mode & 0x10 != 0;
        // The four mode-0 P=1 layouts used by the commercial games.
        let fixed = match mode & 0x2F {
            0x20 | 0x27 => Some([r6 & 0xFE, r6 | 1, r7 & 0xFE, r7 | 1]),
            0x23 | 0x24 => Some([r6 & 0xFE, r7 & 0xFE, r6 | 1, r7 | 1]),
            0x28 | 0x2F => Some([r6 & 0xFE, r6 & 0xFE, r7 & 0xFE, r7 & 0xFE]),
            0x2B | 0x2C => Some([r6 | 1, r7 | 1, r6 | 1, r7 | 1]),
            _ => None,
        };
        if let Some(pages) = fixed {
            if rom {
                return pages[nt];
            }
            // CIRAM: vertical / horizontal / one-screen A / one-screen B.
            return match mode & 0x2F {
                0x20 | 0x27 => [0, 1, 0, 1][nt],
                0x23 | 0x24 => [0, 0, 1, 1][nt],
                0x28 | 0x2F => 0,
                _ => 1,
            };
        }
        match mode & 7 {
            0 | 6 | 7 => [r6, r6, r7, r7][nt],
            1 | 5 => [r[4], r[5], r6, r7][nt],
            _ => [r6, r7, r6, r7][nt],
        }
    }

    fn rom_nametables(&self) -> bool {
        self.mode() & 0x10 != 0 && self.mirroring != Mirroring::FourScreen
    }

    fn nt_rom_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x400;
        let page = self.nametable_page(((addr >> 10) & 3) as usize) as usize % banks;
        page * 0x400 + (addr as usize & 0x3FF)
    }

    /// W (bit 7): PRG RAM at $6000-$7FFF responds only when set.
    fn prg_ram_enabled(&self) -> bool {
        self.mode() & 0x80 != 0
    }

    /// The two register-select bits, with A0/A1 swapped on VRC6b.
    fn reg(&self, addr: u16) -> u16 {
        if self.swap_lines {
            ((addr >> 1) & 1) | ((addr & 1) << 1)
        } else {
            addr & 3
        }
    }
}

impl Mapper for Vrc6 {
    crate::impl_mapper_savestate!(chr, prg_ram);

    fn set_ram_sizes(&mut self, prg_ram: usize, chr_ram: usize) {
        if prg_ram > 0 {
            self.prg_ram = vec![0; prg_ram];
        }
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }
    fn cpu_read(&mut self, addr: u16) -> u8 {
        match addr {
            0x8000..=0xBFFF => {
                let banks = self.prg.len() / 0x4000;
                let bank = self.prg_16k as usize % banks;
                self.prg[bank * 0x4000 + (addr as usize & 0x3FFF)]
            }
            0xC000..=0xDFFF => {
                let banks = self.prg.len() / 0x2000;
                let bank = self.prg_8k as usize % banks;
                self.prg[bank * 0x2000 + (addr as usize & 0x1FFF)]
            }
            0xE000..=0xFFFF => {
                let last = self.prg.len() - 0x2000;
                self.prg[last + (addr as usize & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        // The register-select bits depend on the A0/A1 pinout (VRC6a vs b).
        let r = self.reg(addr);
        match (addr & 0xF000, r) {
            (0x6000 | 0x7000, _) => {
                if self.prg_ram_enabled() {
                    let len = self.prg_ram.len();
                    self.prg_ram[(addr as usize & 0x1FFF) % len] = val;
                }
            }
            (0x8000, _) => self.prg_16k = val & 0x0F,
            (0x9000, 0..=2) => self.audio.pulse_write(0, r, val),
            (0x9000, _) => self.audio.freq_ctrl = val,
            (0xA000, 0..=2) => self.audio.pulse_write(1, r, val),
            (0xB000, 0..=2) => self.audio.saw_write(r, val),
            (0xB000, _) => self.ppu_mode = Some(val),
            (0xC000, _) => self.prg_8k = val & 0x1F,
            (0xD000, k) => self.chr_banks[k as usize] = val,
            (0xE000, k) => self.chr_banks[4 + k as usize] = val,
            (0xF000, 0) => self.irq.latch = val,
            (0xF000, 1) => self.irq.write_control(val),
            (0xF000, 2) => self.irq.ack(),
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x2000 {
            // Routed here by nt_target: CHR-ROM nametables.
            return self.chr[self.nt_rom_offset(addr)];
        }
        self.chr[self.chr_offset(addr)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        // Real VRC6 boards are CHR ROM; the RAM path only serves zero-CHR
        // images (iNES CHR RAM convention).
        if self.chr_is_ram {
            let off = if addr >= 0x2000 {
                self.nt_rom_offset(addr)
            } else {
                self.chr_offset(addr)
            };
            self.chr[off] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        if self.mirroring == Mirroring::FourScreen {
            return Mirroring::FourScreen;
        }
        // Reported for the common CIRAM layouts; nt_target does the routing.
        match self.mode() & 0x2F {
            0x23 | 0x24 => Mirroring::Horizontal,
            0x28 | 0x2F => Mirroring::SingleScreenLo,
            0x2B | 0x2C => Mirroring::SingleScreenHi,
            _ => Mirroring::Vertical,
        }
    }

    fn nt_target(&mut self, addr: u16) -> NtTarget {
        if self.mirroring == Mirroring::FourScreen {
            return NtTarget::Ciram(mirror_nt(Mirroring::FourScreen, addr));
        }
        if self.rom_nametables() {
            return NtTarget::Cart;
        }
        let page = self.nametable_page(((addr >> 10) & 3) as usize) as u16 & 1;
        NtTarget::Ciram((page << 10) | (addr & 0x3FF))
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        if !self.prg_ram_enabled() {
            return None;
        }
        let len = self.prg_ram.len();
        Some(self.prg_ram[(addr as usize & 0x1FFF) % len])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.prg_ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.prg_ram)
    }

    fn irq(&self) -> bool {
        self.irq.line
    }

    fn cpu_clock(&mut self) {
        self.irq.clock();
        self.audio.clock();
    }

    fn audio_sample(&self) -> f32 {
        self.audio.sample()
    }
}

/// The shared Konami VRC IRQ: an up-counter from a reloadable latch, in
/// CPU-cycle mode or scanline mode (a 341/3-dot prescaler).
#[derive(Serialize, Deserialize)]
struct VrcIrq {
    latch: u8,
    counter: u8,
    enabled: bool,
    enable_after_ack: bool,
    cycle_mode: bool,
    prescaler: i16,
    line: bool,
}

impl VrcIrq {
    fn new() -> Self {
        VrcIrq {
            latch: 0,
            counter: 0,
            enabled: false,
            enable_after_ack: false,
            cycle_mode: false,
            prescaler: 341,
            line: false,
        }
    }

    fn write_control(&mut self, val: u8) {
        self.enable_after_ack = val & 1 != 0;
        self.enabled = val & 2 != 0;
        self.cycle_mode = val & 4 != 0;
        self.line = false;
        if self.enabled {
            self.counter = self.latch;
            self.prescaler = 341;
        }
    }

    fn ack(&mut self) {
        self.line = false;
        self.enabled = self.enable_after_ack;
    }

    fn clock(&mut self) {
        if !self.enabled {
            return;
        }
        if !self.cycle_mode {
            // Scanline mode: one step per 113.667 CPU cycles.
            self.prescaler -= 3;
            if self.prescaler > 0 {
                return;
            }
            self.prescaler += 341;
        }
        if self.counter == 0xFF {
            self.counter = self.latch;
            self.line = true;
        } else {
            self.counter += 1;
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Vrc6Pulse {
    // $x000: bits 0-3 volume, 4-6 duty, 7 constant-output mode.
    ctrl: u8,
    period: u16,
    enabled: bool,
    counter: u16,
    step: u8,
}

impl Vrc6Pulse {
    fn output(&self) -> u8 {
        if !self.enabled {
            0
        } else if self.ctrl & 0x80 != 0 || self.step <= (self.ctrl >> 4) & 7 {
            self.ctrl & 0x0F
        } else {
            0
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Vrc6Audio {
    pulses: [Vrc6Pulse; 2],
    saw_rate: u8,
    saw_period: u16,
    saw_enabled: bool,
    saw_counter: u16,
    saw_step: u8,
    saw_acc: u8,
    // $9003: bit 0 halts all channels, bits 1-2 shift the periods.
    freq_ctrl: u8,
}

impl Vrc6Audio {
    pub(crate) fn new() -> Self {
        Vrc6Audio {
            pulses: [
                Vrc6Pulse {
                    ctrl: 0,
                    period: 0,
                    enabled: false,
                    counter: 0,
                    step: 0,
                },
                Vrc6Pulse {
                    ctrl: 0,
                    period: 0,
                    enabled: false,
                    counter: 0,
                    step: 0,
                },
            ],
            saw_rate: 0,
            saw_period: 0,
            saw_enabled: false,
            saw_counter: 0,
            saw_step: 0,
            saw_acc: 0,
            freq_ctrl: 0,
        }
    }

    /// Register write by VRC6a address ($9000-$9003, $A000-$A002,
    /// $B000-$B002) - the layout NSF files use. Other addresses are ignored.
    pub(crate) fn write(&mut self, addr: u16, val: u8) {
        let r = addr & 3;
        match (addr & 0xF000, r) {
            (0x9000, 0..=2) => self.pulse_write(0, r, val),
            (0x9000, _) => self.freq_ctrl = val,
            (0xA000, 0..=2) => self.pulse_write(1, r, val),
            (0xB000, 0..=2) => self.saw_write(r, val),
            _ => {}
        }
    }

    fn pulse_write(&mut self, ch: usize, reg: u16, val: u8) {
        let p = &mut self.pulses[ch];
        match reg {
            0 => p.ctrl = val,
            1 => p.period = (p.period & 0xF00) | val as u16,
            _ => {
                p.period = (p.period & 0x0FF) | ((val as u16 & 0x0F) << 8);
                p.enabled = val & 0x80 != 0;
                if !p.enabled {
                    p.step = 0;
                }
            }
        }
    }

    fn saw_write(&mut self, reg: u16, val: u8) {
        match reg {
            0 => self.saw_rate = val & 0x3F,
            1 => self.saw_period = (self.saw_period & 0xF00) | val as u16,
            _ => {
                self.saw_period = (self.saw_period & 0x0FF) | ((val as u16 & 0x0F) << 8);
                self.saw_enabled = val & 0x80 != 0;
                if !self.saw_enabled {
                    self.saw_acc = 0;
                    self.saw_step = 0;
                }
            }
        }
    }

    fn effective_period(&self, period: u16) -> u16 {
        if self.freq_ctrl & 4 != 0 {
            period >> 8
        } else if self.freq_ctrl & 2 != 0 {
            period >> 4
        } else {
            period
        }
    }

    pub(crate) fn clock(&mut self) {
        if self.freq_ctrl & 1 != 0 {
            return; // halted
        }
        for ch in 0..2 {
            let eff = self.effective_period(self.pulses[ch].period);
            let p = &mut self.pulses[ch];
            if !p.enabled {
                continue;
            }
            if p.counter == 0 {
                p.counter = eff;
                p.step = (p.step + 1) & 15;
            } else {
                p.counter -= 1;
            }
        }
        if self.saw_enabled {
            if self.saw_counter == 0 {
                self.saw_counter = self.effective_period(self.saw_period);
                self.saw_step += 1;
                // The accumulator adds on every other step; 14 steps per cycle.
                if self.saw_step & 1 == 0 {
                    self.saw_acc = self.saw_acc.wrapping_add(self.saw_rate);
                }
                if self.saw_step == 14 {
                    self.saw_step = 0;
                    self.saw_acc = 0;
                }
            } else {
                self.saw_counter -= 1;
            }
        }
    }

    pub(crate) fn sample(&self) -> f32 {
        let p = self.pulses[0].output() + self.pulses[1].output();
        let saw = self.saw_acc >> 3; // top 5 bits
        (p + saw) as f32 * 0.00752
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vrc6() -> Vrc6 {
        // 8 x 16KB PRG, 16 x 1KB CHR; byte = bank index.
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Vrc6::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_banking() {
        let mut m = vrc6();
        m.cpu_write(0x8000, 2);
        m.cpu_write(0xC000, 9); // 8KB banks: 16KB bank 4 second half
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xC000), 4); // 8KB bank 9 lives in 16KB bank 4
        assert_eq!(m.cpu_read(0xE000), 7); // fixed last
    }

    #[test]
    fn chr_banking() {
        let mut m = vrc6();
        m.cpu_write(0xD002, 11);
        m.cpu_write(0xE001, 3);
        assert_eq!(m.ppu_read(0x0800), 11);
        assert_eq!(m.ppu_read(0x1400), 3);
    }

    #[test]
    fn vrc6b_swaps_a0_a1() {
        // 8 x 16KB PRG; on VRC6b the $C000 8KB-bank register decodes at A1
        // swapped with A0, so a write to $C001 hits it (A1=0, A0=1 -> reg 2
        // becomes reg 1's slot). Verify PRG select still lands via the b pinout.
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        let mut m = Vrc6::new_vrc6b(prg, chr, Mirroring::Vertical);
        // $8000 (reg 0) selects the 16KB bank regardless of pinout.
        m.cpu_write(0x8000, 2);
        assert_eq!(m.cpu_read(0x8000), 2);
        // Mirroring control is at ($B000, reg 3). On VRC6b reg 3 needs A0=1,
        // A1=1 with the lines swapped, i.e. address bits 1,0 = 1,1 -> $B003.
        m.cpu_write(0xB003, 0x24);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        // The CHR low-bank register ($D000 reg group): on VRC6b $D001 and
        // $D002 swap. Writing $D002 targets reg index 1.
        m.cpu_write(0xD002, 9);
        assert_eq!(m.ppu_read(0x0400), 9);
    }

    /// CIRAM page (0/1) for each of the four nametables.
    fn ciram_pages(m: &mut Vrc6) -> [u16; 4] {
        [0x2000u16, 0x2400, 0x2800, 0x2C00].map(|a| match m.nt_target(a) {
            NtTarget::Ciram(off) => off >> 10,
            NtTarget::Cart => 9,
        })
    }

    #[test]
    fn mirroring_bits() {
        // The values the commercial games write: mode 0, P=1, CIRAM.
        let mut m = vrc6();
        for (val, pages, mir) in [
            (0x20, [0, 1, 0, 1], Mirroring::Vertical),
            (0x24, [0, 0, 1, 1], Mirroring::Horizontal),
            (0x28, [0, 0, 0, 0], Mirroring::SingleScreenLo),
            (0xAC, [1, 1, 1, 1], Mirroring::SingleScreenHi),
        ] {
            m.cpu_write(0xB003, val);
            assert_eq!(m.mirroring(), mir, "${val:02X}");
            assert_eq!(ciram_pages(&mut m), pages, "${val:02X}");
        }
    }

    #[test]
    fn ciram_pages_from_registers_outside_the_fixed_layouts() {
        let mut m = vrc6();
        m.cpu_write(0xE000, 0x11); // R4 odd
        m.cpu_write(0xE001, 0x10); // R5 even
        m.cpu_write(0xE002, 0x03); // R6 odd
        m.cpu_write(0xE003, 0x02); // R7 even
        m.cpu_write(0xB003, 0x01); // mode 1, P=0: R4-R7 select each NT
        assert_eq!(ciram_pages(&mut m), [1, 0, 1, 0]);
        m.cpu_write(0xB003, 0x02); // mode 2: R6 R7 R6 R7
        assert_eq!(ciram_pages(&mut m), [1, 0, 1, 0]);
        m.cpu_write(0xB003, 0x00); // mode 0, P=0: R6 R6 R7 R7
        assert_eq!(ciram_pages(&mut m), [1, 1, 0, 0]);
    }

    #[test]
    fn chr_modes_and_a10_rule() {
        let mut m = vrc6();
        for (i, v) in [2u8, 5, 8, 11, 12, 15, 6, 7].iter().enumerate() {
            let a = [
                0xD000, 0xD001, 0xD002, 0xD003, 0xE000, 0xE001, 0xE002, 0xE003,
            ][i];
            m.cpu_write(a, *v);
        }
        let pages = |m: &mut Vrc6| -> Vec<u8> { (0..8).map(|s| m.ppu_read(s * 0x400)).collect() };
        m.cpu_write(0xB003, 0x20); // mode 0: eight 1KB registers
        assert_eq!(pages(&mut m), [2, 5, 8, 11, 12, 15, 6, 7]);
        m.cpu_write(0xB003, 0x21); // mode 1, P=1: R0-R3 as 2KB (A10 from PPU)
        assert_eq!(pages(&mut m), [2, 3, 4, 5, 8, 9, 10, 11]);
        m.cpu_write(0xB003, 0x01); // mode 1, P=0: A10 from the register LSB
        assert_eq!(pages(&mut m), [2, 2, 5, 5, 8, 8, 11, 11]);
        m.cpu_write(0xB003, 0x22); // mode 2: 1KB low half, 2KB high half
        assert_eq!(pages(&mut m), [2, 5, 8, 11, 12, 13, 14, 15]);
        m.cpu_write(0xB003, 0x03); // mode 3, P=0
        assert_eq!(pages(&mut m), [2, 5, 8, 11, 12, 12, 15, 15]);
    }

    #[test]
    fn rom_nametables() {
        let mut m = vrc6();
        m.cpu_write(0xE000, 9); // R4
        m.cpu_write(0xE001, 10); // R5
        m.cpu_write(0xE002, 13); // R6
        m.cpu_write(0xE003, 14); // R7
        m.cpu_write(0xB003, 0x11); // mode 1, N=1: four-screen from R4-R7
        assert_eq!(m.nt_target(0x2000), NtTarget::Cart);
        let nts: Vec<u8> = [0x2000, 0x2400, 0x2800, 0x2C00]
            .iter()
            .map(|&a| m.ppu_read(a))
            .collect();
        assert_eq!(nts, [9, 10, 13, 14]);
        m.cpu_write(0xB003, 0x34); // mode 0, P=1, N=1, MM=1: R6/R7 pairs
        let nts: Vec<u8> = [0x2000, 0x2400, 0x2800, 0x2C00]
            .iter()
            .map(|&a| m.ppu_read(a))
            .collect();
        assert_eq!(nts, [12, 14, 13, 15]);
    }

    #[test]
    fn prg_ram_needs_enable_bit() {
        let mut m = vrc6();
        m.cpu_write(0x6000, 0x55);
        assert_eq!(m.prg_ram_read(0x6000), None, "disabled at power-on");
        m.cpu_write(0xB003, 0xA0);
        m.cpu_write(0x6000, 0x55);
        assert_eq!(m.prg_ram_read(0x6000), Some(0x55));
        m.cpu_write(0xB003, 0x20);
        assert_eq!(m.prg_ram_read(0x6000), None);
        m.cpu_write(0x7FFF, 1); // ignored while disabled
        m.cpu_write(0xB003, 0xA0);
        assert_eq!(m.prg_ram_read(0x7FFF), Some(0));
    }

    #[test]
    fn legacy_savestate_keeps_its_mirroring() {
        let mut m = vrc6();
        m.cpu_write(0xB003, 0x24);
        let mut state: serde_json::Value = serde_json::from_slice(&m.save_state()).unwrap();
        // States from before $B003 was modelled: no ppu_mode, mirroring set.
        state.as_object_mut().unwrap().remove("ppu_mode");
        state["mirroring"] = serde_json::json!("Horizontal");
        let mut restored = vrc6();
        restored
            .load_state(&serde_json::to_vec(&state).unwrap())
            .unwrap();
        assert_eq!(restored.mirroring(), Mirroring::Horizontal);
        restored.cpu_write(0x6000, 7);
        assert_eq!(restored.prg_ram_read(0x6000), Some(7), "RAM stays usable");
    }

    #[test]
    fn irq_cycle_mode() {
        let mut m = vrc6();
        m.cpu_write(0xF000, 0xFD); // latch: 3 steps to $FF
        m.cpu_write(0xF001, 0x06); // enable, cycle mode
        for i in 0..2 {
            m.cpu_clock();
            assert!(!m.irq(), "IRQ too early at cycle {i}");
        }
        m.cpu_clock(); // $FF -> reload, IRQ
        assert!(m.irq());
        // $F002 acks; enable-after-ack clear stops the counter.
        m.cpu_write(0xF002, 0);
        assert!(!m.irq());
        for _ in 0..1000 {
            m.cpu_clock();
        }
        assert!(!m.irq());
    }

    #[test]
    fn irq_scanline_mode_cadence() {
        let mut m = vrc6();
        m.cpu_write(0xF000, 0xFF); // IRQ on first counter step
        m.cpu_write(0xF001, 0x02); // enable, scanline mode
        // First step lands after ceil(341/3) = 114 CPU cycles.
        for _ in 0..113 {
            m.cpu_clock();
        }
        assert!(!m.irq());
        m.cpu_clock();
        assert!(m.irq());
    }

    #[test]
    fn pulse_and_saw_output() {
        let mut m = vrc6();
        m.cpu_write(0x9000, 0x0F); // duty 0, volume 15
        m.cpu_write(0x9001, 0x04);
        m.cpu_write(0x9002, 0x80); // enable
        m.cpu_write(0xB000, 0x20); // saw rate
        m.cpu_write(0xB001, 0x04);
        m.cpu_write(0xB002, 0x80); // enable
        let mut peak = 0.0f32;
        for _ in 0..500 {
            m.cpu_clock();
            peak = peak.max(m.audio_sample());
        }
        assert!(peak > 0.05, "expected VRC6 audio output, got {peak}");
        // Halt bit freezes the channels.
        m.cpu_write(0x9003, 0x01);
        let s0 = m.audio_sample();
        for _ in 0..100 {
            m.cpu_clock();
        }
        assert_eq!(m.audio_sample(), s0);
    }
}
