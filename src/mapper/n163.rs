use super::Chr;
use super::{Mapper, Mirroring, NtTarget, mirror_nt};
use serde::{Deserialize, Serialize};

/// Namco 163 (mapper 19, Rolling Thunder, Megami Tensei II): 1KB CHR
/// banking that extends into nametable space, a 15-bit CPU-cycle IRQ
/// counter, and a multiplexed wavetable sound chip with up to 8 channels.
///
/// Any CHR register holding $E0-$FF selects a 1KB CIRAM page (bit 0)
/// instead of CHR ROM - always for the four nametable registers, and for
/// the pattern registers unless $E800 bit 6 ($0000-$0FFF) / bit 7
/// ($1000-$1FFF) is set. Because pattern fetches can land in CIRAM, the
/// board owns the 2KB of nametable RAM itself (every nametable access is
/// routed here). $E000 bit 6 mutes the sound chip and $F800 doubles as the
/// PRG RAM write protect (upper nibble $4 = writable, bits 0-3 protect
/// each 2KB window).
#[derive(Serialize, Deserialize)]
pub struct N163 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Chr,
    #[serde(default)]
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    prg_banks: [u8; 3],
    // CHR regs for $0000-$1FFF (8) and the four nametables (4). Values
    // $E0-$FF select a CIRAM page instead of CHR ROM.
    chr_banks: [u8; 12],
    /// The console's 2KB nametable RAM, served by the board.
    #[serde(default = "ciram_default")]
    ciram: Vec<u8>,
    /// $E800 bits 6/7: pattern registers $E0+ read CHR ROM, not CIRAM.
    #[serde(default)]
    chr_rom_low: bool,
    #[serde(default)]
    chr_rom_high: bool,
    /// $E000 bit 6: sound chip muted.
    #[serde(default)]
    sound_disabled: bool,
    /// Last $F800 value (PRG RAM write protect). Until the game first writes
    /// $F800 the RAM stays writable, so iNES dumps of Namco 175 boards
    /// mislabeled as mapper 19 (which have no such register) keep saving.
    #[serde(default)]
    write_protect: Option<u8>,
    irq_counter: u16,
    irq_enabled: bool,
    irq_line: bool,
    audio: N163Audio,
}

fn ciram_default() -> Vec<u8> {
    vec![0; 0x800]
}

impl N163 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        // The four nametable registers power up selecting CIRAM pages that
        // match the header mirroring (a value of $E0|page). True N163 games
        // overwrite these during init, so this only matters for boards that
        // never touch them - notably Namco 175/340 carts (mapper 210) that are
        // mislabeled as plain mapper 19 in iNES 1.0 dumps, where leaving the
        // registers at 0 would wrongly fetch nametables from CHR ROM.
        let nt = |i: u16| 0xE0 | ((mirror_nt(mirroring, 0x2000 + i * 0x400) >> 10) & 1) as u8;
        let chr_banks = [0, 0, 0, 0, 0, 0, 0, 0, nt(0), nt(1), nt(2), nt(3)];
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        N163 {
            prg,
            chr: chr.into(),
            chr_is_ram,
            prg_ram: vec![0; 0x2000],
            prg_banks: [0; 3],
            chr_banks,
            ciram: ciram_default(),
            chr_rom_low: false,
            chr_rom_high: false,
            sound_disabled: false,
            write_protect: None,
            irq_counter: 0,
            irq_enabled: false,
            irq_line: false,
            audio: N163Audio::new(),
        }
    }

    fn chr_offset(&self, bank: u8, addr: u16) -> usize {
        let banks = self.chr.len() / 0x400;
        (bank as usize % banks) * 0x400 + (addr as usize & 0x3FF)
    }

    /// CIRAM offset when `addr`'s register selects a CIRAM page.
    fn ciram_offset(&self, addr: u16) -> Option<usize> {
        let bank = self.chr_bank_for(addr);
        if bank < 0xE0 {
            return None;
        }
        if addr < 0x2000 {
            let rom = if addr < 0x1000 {
                self.chr_rom_low
            } else {
                self.chr_rom_high
            };
            if rom {
                return None;
            }
        }
        Some(((bank as usize & 1) << 10) | (addr as usize & 0x3FF))
    }

    fn prg_ram_writable(&self, addr: u16) -> bool {
        match self.write_protect {
            None => true,
            Some(wp) => wp & 0xF0 == 0x40 && wp & (1 << ((addr >> 11) & 3)) == 0,
        }
    }

    /// CHR bank register backing a PPU address: pattern space uses the
    /// eight $0000-$1FFF registers, nametable space the four NT registers.
    fn chr_bank_for(&self, addr: u16) -> u8 {
        if addr < 0x2000 {
            self.chr_banks[(addr >> 10) as usize & 7]
        } else {
            self.chr_banks[8 + ((addr >> 10) & 3) as usize]
        }
    }
}

impl Mapper for N163 {
    crate::impl_mapper_savestate!(chr, prg_ram, ciram);

    fn set_ram_sizes(&mut self, prg_ram: usize, chr_ram: usize) {
        if prg_ram > 0 {
            self.prg_ram = vec![0; prg_ram];
        }
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram].into();
        }
    }
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let banks = self.prg.len() / 0x2000;
        let bank = match addr {
            0x8000..=0x9FFF => self.prg_banks[0] as usize % banks,
            0xA000..=0xBFFF => self.prg_banks[1] as usize % banks,
            0xC000..=0xDFFF => self.prg_banks[2] as usize % banks,
            _ => banks - 1,
        };
        self.prg[bank * 0x2000 + (addr as usize & 0x1FFF)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr & 0xF800 {
            0x4800 => self.audio.data_write(val),
            0x5000 => {
                self.irq_counter = (self.irq_counter & 0x7F00) | val as u16;
                self.irq_line = false;
            }
            0x5800 => {
                self.irq_counter = (self.irq_counter & 0x00FF) | ((val as u16 & 0x7F) << 8);
                self.irq_enabled = val & 0x80 != 0;
                self.irq_line = false;
            }
            0x6000..=0x7800 => {
                if self.prg_ram_writable(addr) {
                    let len = self.prg_ram.len();
                    self.prg_ram[(addr as usize & 0x1FFF) % len] = val;
                }
            }
            0x8000..=0xB800 => self.chr_banks[((addr - 0x8000) >> 11) as usize] = val,
            0xC000..=0xD800 => self.chr_banks[8 + ((addr - 0xC000) >> 11) as usize] = val,
            0xE000 => {
                self.prg_banks[0] = val & 0x3F;
                self.sound_disabled = val & 0x40 != 0;
            }
            0xE800 => {
                self.prg_banks[1] = val & 0x3F;
                self.chr_rom_low = val & 0x40 != 0;
                self.chr_rom_high = val & 0x80 != 0;
            }
            0xF000 => self.prg_banks[2] = val & 0x3F,
            0xF800 => {
                self.write_protect = Some(val);
                self.audio.addr_write(val);
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        match self.ciram_offset(addr) {
            Some(off) => self.ciram[off],
            None => self.chr[self.chr_offset(self.chr_bank_for(addr), addr)],
        }
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if let Some(off) = self.ciram_offset(addr) {
            self.ciram[off] = val;
        } else if self.chr_is_ram {
            // Real N163 boards are CHR ROM; the RAM path only serves
            // zero-CHR images (iNES CHR RAM convention).
            let off = self.chr_offset(self.chr_bank_for(addr), addr);
            self.chr[off] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        // Unused: nt_target below overrides nametable routing.
        Mirroring::Vertical
    }

    fn nt_target(&mut self, _addr: u16) -> NtTarget {
        // The board serves CIRAM itself (pattern fetches can map it too).
        NtTarget::Cart
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        Some(self.prg_ram[(addr & 0x1FFF) as usize])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.prg_ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.prg_ram)
    }

    fn cpu_reg_read(&mut self, addr: u16) -> Option<u8> {
        match addr & 0xF800 {
            0x4800 => Some(self.audio.data_read()),
            0x5000 => Some(self.irq_counter as u8),
            0x5800 => Some((self.irq_counter >> 8) as u8 | if self.irq_enabled { 0x80 } else { 0 }),
            _ => None,
        }
    }

    fn irq(&self) -> bool {
        self.irq_line
    }

    fn cpu_clock(&mut self) {
        if self.irq_enabled && self.irq_counter < 0x7FFF {
            self.irq_counter += 1;
            if self.irq_counter == 0x7FFF {
                self.irq_line = true;
            }
        }
        self.audio.clock();
    }

    fn audio_sample(&self) -> f32 {
        if self.sound_disabled {
            0.0
        } else {
            self.audio.sample()
        }
    }
}

/// N163 sound: 128 bytes of internal RAM holding packed 4-bit wavetables
/// and, in the top bytes, the channel registers. The hardware updates one
/// enabled channel every 15 CPU cycles, round-robin from channel 7 down.
#[derive(Serialize, Deserialize)]
pub(crate) struct N163Audio {
    #[serde(with = "crate::savestate::byte_array")]
    ram: [u8; 0x80],
    addr: u8,
    auto_inc: bool,
    divider: u8,
    cur: usize,
    // Last computed output per channel, in (sample-8)*vol units.
    outputs: [f32; 8],
}

impl N163Audio {
    pub(crate) fn new() -> Self {
        N163Audio {
            ram: [0; 0x80],
            addr: 0,
            auto_inc: false,
            divider: 0,
            cur: 0,
            outputs: [0.0; 8],
        }
    }

    pub(crate) fn addr_write(&mut self, val: u8) {
        self.addr = val & 0x7F;
        self.auto_inc = val & 0x80 != 0;
    }

    pub(crate) fn data_write(&mut self, val: u8) {
        self.ram[self.addr as usize] = val;
        if self.auto_inc {
            self.addr = (self.addr + 1) & 0x7F;
        }
    }

    pub(crate) fn data_read(&mut self) -> u8 {
        let v = self.ram[self.addr as usize];
        if self.auto_inc {
            self.addr = (self.addr + 1) & 0x7F;
        }
        v
    }

    fn enabled_channels(&self) -> usize {
        ((self.ram[0x7F] >> 4) & 7) as usize + 1
    }

    /// Advance one channel's 18-bit phase accumulator and sample its wave.
    fn update_channel(&mut self, ch: usize) {
        let base = 0x40 + ch * 8;
        let freq = (self.ram[base] as u32)
            | ((self.ram[base + 2] as u32) << 8)
            | ((self.ram[base + 4] as u32 & 3) << 16);
        let length = (256 - (self.ram[base + 4] as u32 & 0xFC)).max(4);
        let mut phase = (self.ram[base + 1] as u32)
            | ((self.ram[base + 3] as u32) << 8)
            | ((self.ram[base + 5] as u32) << 16);
        phase = (phase + freq) % (length << 16);
        self.ram[base + 1] = phase as u8;
        self.ram[base + 3] = (phase >> 8) as u8;
        self.ram[base + 5] = (phase >> 16) as u8;
        let index = (self.ram[base + 6] as u32 + (phase >> 16)) & 0xFF;
        let byte = self.ram[(index as usize / 2) & 0x7F];
        let sample = if index & 1 != 0 {
            byte >> 4
        } else {
            byte & 0x0F
        };
        let vol = self.ram[base + 7] & 0x0F;
        self.outputs[ch] = (sample as f32 - 8.0) / 8.0 * (vol as f32 / 15.0);
    }

    pub(crate) fn clock(&mut self) {
        self.divider += 1;
        if self.divider < 15 {
            return;
        }
        self.divider = 0;
        let n = self.enabled_channels();
        self.cur = (self.cur + 1) % n;
        let ch = 7 - self.cur;
        self.update_channel(ch);
    }

    pub(crate) fn sample(&self) -> f32 {
        let n = self.enabled_channels();
        let s: f32 = self.outputs[8 - n..].iter().sum();
        // The chip time-multiplexes its DAC, so loudness stays roughly
        // constant regardless of the channel count.
        s * 0.12 / n as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n163() -> N163 {
        // 8 x 8KB PRG, 64 x 1KB CHR; byte = bank index.
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..64 * 0x400).map(|i| (i / 0x400) as u8).collect();
        N163::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_banking() {
        let mut m = n163();
        m.cpu_write(0xE000, 3);
        m.cpu_write(0xE800, 4);
        m.cpu_write(0xF000, 5);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xA000), 4);
        assert_eq!(m.cpu_read(0xC000), 5);
        assert_eq!(m.cpu_read(0xE000), 7);
    }

    #[test]
    fn chr_banking() {
        let mut m = n163();
        m.cpu_write(0x8000, 33);
        m.cpu_write(0xB800, 7);
        assert_eq!(m.ppu_read(0x0000), 33);
        assert_eq!(m.ppu_read(0x1C00), 7);
    }

    #[test]
    fn nametables_route_to_ciram_or_chr() {
        let mut m = n163();
        m.cpu_write(0xC000, 0xE0); // NT0 -> CIRAM page 0
        m.cpu_write(0xC800, 0xE1); // NT1 -> CIRAM page 1
        m.cpu_write(0xD000, 12); // NT2 -> CHR ROM bank 12
        m.cpu_write(0xD800, 0xE0); // NT3 -> CIRAM page 0 (shared with NT0)
        assert_eq!(m.nt_target(0x2005), NtTarget::Cart);
        m.ppu_write(0x2005, 0xAA);
        m.ppu_write(0x2405, 0xBB);
        assert_eq!(m.ppu_read(0x2C05), 0xAA, "NT3 mirrors CIRAM page 0");
        assert_eq!(m.ppu_read(0x2405), 0xBB);
        assert_eq!(m.ppu_read(0x2800), 12);
        m.ppu_write(0x2800, 0x55); // CHR ROM nametable: read-only
        assert_eq!(m.ppu_read(0x2800), 12);
    }

    #[test]
    fn pattern_banks_e0_and_up_map_ciram_unless_e800_disables_it() {
        let mut m = n163();
        m.cpu_write(0xC000, 0xE1); // NT0 -> CIRAM page 1
        m.ppu_write(0x2010, 0x77);
        m.cpu_write(0x8000, 0xE1); // $0000 pattern slot -> CIRAM page 1
        m.cpu_write(0xA000, 0xE1); // $1000 pattern slot -> CIRAM page 1
        assert_eq!(m.ppu_read(0x0010), 0x77);
        assert_eq!(m.ppu_read(0x1010), 0x77);
        m.ppu_write(0x0011, 0x66); // CIRAM is writable through CHR space
        assert_eq!(m.ppu_read(0x2011), 0x66);
        // $E800 bit 6: low pattern table reads CHR ROM; bit 7: high one.
        m.cpu_write(0xE800, 0x40);
        assert_eq!(m.ppu_read(0x0010), 0xE1 % 64);
        assert_eq!(m.ppu_read(0x1010), 0x77);
        m.cpu_write(0xE800, 0xC0);
        assert_eq!(m.ppu_read(0x1010), 0xE1 % 64);
        // Nametable registers have no disable bit.
        assert_eq!(m.ppu_read(0x2010), 0x77);
    }

    #[test]
    fn sound_disable_bit_mutes_output() {
        let mut m = n163();
        m.cpu_write(0xF800, 0xF8); // addr $78, auto-inc
        for v in [0, 0, 0x40, 0, 0xF8, 0, 0, 0x0F] {
            m.cpu_write(0x4800, v);
        }
        m.cpu_write(0xF800, 0x80);
        for _ in 0..4 {
            m.cpu_write(0x4800, 0xFF);
        }
        let peak = |m: &mut N163| {
            (0..2000).fold(0.0f32, |p, _| {
                m.cpu_clock();
                p.max(m.audio_sample().abs())
            })
        };
        assert!(peak(&mut m) > 0.0);
        m.cpu_write(0xE000, 0x40);
        assert_eq!(peak(&mut m), 0.0);
        m.cpu_write(0xE000, 0x00);
        assert!(peak(&mut m) > 0.0);
    }

    #[test]
    fn f800_write_protects_prg_ram() {
        let mut m = n163();
        m.cpu_write(0x6000, 1); // no $F800 write yet: writable
        assert_eq!(m.prg_ram_read(0x6000), Some(1));
        m.cpu_write(0xF800, 0x00); // upper nibble != 4: all protected
        m.cpu_write(0x6000, 2);
        assert_eq!(m.prg_ram_read(0x6000), Some(1));
        m.cpu_write(0xF800, 0x42); // writable except window 1 ($6800-$6FFF)
        m.cpu_write(0x6000, 3);
        m.cpu_write(0x6800, 4);
        m.cpu_write(0x7800, 5);
        assert_eq!(m.prg_ram_read(0x6000), Some(3));
        assert_eq!(m.prg_ram_read(0x6800), Some(0));
        assert_eq!(m.prg_ram_read(0x7800), Some(5));
    }

    #[test]
    fn ciram_survives_savestate() {
        let mut m = n163();
        m.cpu_write(0xC000, 0xE0);
        m.ppu_write(0x2123, 0x9C);
        let blob = m.save_state();
        let mut r = n163();
        r.load_state(&blob).unwrap();
        r.cpu_write(0xC000, 0xE0);
        assert_eq!(r.ppu_read(0x2123), 0x9C);
    }

    #[test]
    fn irq_counts_up_and_reads_back() {
        let mut m = n163();
        m.cpu_write(0x5000, 0xFC); // counter = $7FFC
        m.cpu_write(0x5800, 0xFF); // high bits + enable
        for i in 0..2 {
            m.cpu_clock();
            assert!(!m.irq(), "IRQ too early at cycle {i}");
        }
        m.cpu_clock(); // reaches $7FFF
        assert!(m.irq());
        assert_eq!(m.cpu_reg_read(0x5000), Some(0xFF));
        // Writing the counter acknowledges.
        m.cpu_write(0x5000, 0x00);
        assert!(!m.irq());
    }

    #[test]
    fn sound_ram_auto_increment() {
        let mut m = n163();
        m.cpu_write(0xF800, 0x80); // addr 0, auto-inc
        m.cpu_write(0x4800, 0x12);
        m.cpu_write(0x4800, 0x34);
        m.cpu_write(0xF800, 0x00);
        assert_eq!(m.cpu_reg_read(0x4800), Some(0x12));
        m.cpu_write(0xF800, 0x01);
        assert_eq!(m.cpu_reg_read(0x4800), Some(0x34));
    }

    #[test]
    fn wavetable_produces_output() {
        let mut m = n163();
        // One enabled channel (channel 7, regs at $78-$7F).
        m.cpu_write(0xF800, 0x80);
        // Wave at RAM 0: 8 samples alternating 0/15 (packed two per byte).
        for _ in 0..4 {
            m.cpu_write(0x4800, 0xF0);
        }
        m.cpu_write(0xF800, 0x78 | 0x80);
        m.cpu_write(0x4800, 0x00); // freq low
        m.cpu_write(0x4800, 0x00); // phase
        m.cpu_write(0x4800, 0x40); // freq mid
        m.cpu_write(0x4800, 0x00); // phase
        m.cpu_write(0x4800, 0xF8); // freq hi 0, length 256-248=8
        m.cpu_write(0x4800, 0x00); // phase
        m.cpu_write(0x4800, 0x00); // wave address 0
        m.cpu_write(0x4800, 0x0F); // volume 15, 1 channel
        let mut peak = 0.0f32;
        for _ in 0..10_000 {
            m.cpu_clock();
            peak = peak.max(m.audio_sample().abs());
        }
        assert!(peak > 0.01, "expected N163 audio output, got {peak}");
    }
}
