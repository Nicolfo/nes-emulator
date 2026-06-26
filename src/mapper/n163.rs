use super::{Mapper, Mirroring, NtTarget, mirror_nt};
use serde::{Deserialize, Serialize};

/// Namco 163 (mapper 19, Rolling Thunder, Megami Tensei II): 1KB CHR
/// banking that extends into nametable space, a 15-bit CPU-cycle IRQ
/// counter, and a multiplexed wavetable sound chip with up to 8 channels.
#[derive(Serialize, Deserialize)]
pub struct N163 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    #[serde(with = "crate::savestate::byte_array")]
    prg_ram: [u8; 0x2000],
    prg_banks: [u8; 3],
    // CHR regs for $0000-$1FFF (8) and the four nametables (4). Values
    // $E0-$FF select a CIRAM page instead of CHR ROM.
    chr_banks: [u8; 12],
    irq_counter: u16,
    irq_enabled: bool,
    irq_line: bool,
    audio: N163Audio,
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
        N163 {
            prg,
            chr,
            prg_ram: [0; 0x2000],
            prg_banks: [0; 3],
            chr_banks,
            irq_counter: 0,
            irq_enabled: false,
            irq_line: false,
            audio: N163Audio::new(),
        }
    }

    fn chr_byte(&self, bank: u8, addr: u16) -> u8 {
        let banks = self.chr.len() / 0x400;
        self.chr[(bank as usize % banks) * 0x400 + (addr as usize & 0x3FF)]
    }
}

impl Mapper for N163 {
    crate::impl_mapper_savestate!();
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
            0x6000..=0x7800 => self.prg_ram[(addr & 0x1FFF) as usize] = val,
            0x8000..=0xB800 => self.chr_banks[((addr - 0x8000) >> 11) as usize] = val,
            0xC000..=0xD800 => self.chr_banks[8 + ((addr - 0xC000) >> 11) as usize] = val,
            0xE000 => self.prg_banks[0] = val & 0x3F,
            // Bits 6/7 ($E800) disable CIRAM selection per pattern table;
            // not emulated (pattern fetches always serve CHR ROM here).
            0xE800 => self.prg_banks[1] = val & 0x3F,
            0xF000 => self.prg_banks[2] = val & 0x3F,
            0xF800 => self.audio.addr_write(val),
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x2000 {
            // Banks $E0+ can map CIRAM into pattern space (unless disabled
            // by $E800 bits 6/7); no supported game uses that, serve ROM.
            let bank = self.chr_banks[(addr >> 10) as usize & 7];
            self.chr_byte(bank, addr)
        } else {
            // Nametable routed here when the NT register selects CHR ROM.
            let bank = self.chr_banks[8 + ((addr >> 10) & 3) as usize];
            self.chr_byte(bank, addr)
        }
    }

    fn ppu_write(&mut self, _addr: u16, _val: u8) {
        // CHR is ROM; CIRAM nametable writes go through NtTarget::Ciram.
    }

    fn mirroring(&self) -> Mirroring {
        // Unused: nt_target below overrides nametable routing.
        Mirroring::Vertical
    }

    fn nt_target(&mut self, addr: u16) -> NtTarget {
        let bank = self.chr_banks[8 + ((addr >> 10) & 3) as usize];
        if bank >= 0xE0 {
            NtTarget::Ciram(((bank as u16 & 1) << 10) | (addr & 0x3FF))
        } else {
            NtTarget::Cart
        }
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
        self.audio.sample()
    }
}

/// N163 sound: 128 bytes of internal RAM holding packed 4-bit wavetables
/// and, in the top bytes, the channel registers. The hardware updates one
/// enabled channel every 15 CPU cycles, round-robin from channel 7 down.
#[derive(Serialize, Deserialize)]
struct N163Audio {
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
    fn new() -> Self {
        N163Audio {
            ram: [0; 0x80],
            addr: 0,
            auto_inc: false,
            divider: 0,
            cur: 0,
            outputs: [0.0; 8],
        }
    }

    fn addr_write(&mut self, val: u8) {
        self.addr = val & 0x7F;
        self.auto_inc = val & 0x80 != 0;
    }

    fn data_write(&mut self, val: u8) {
        self.ram[self.addr as usize] = val;
        if self.auto_inc {
            self.addr = (self.addr + 1) & 0x7F;
        }
    }

    fn data_read(&mut self) -> u8 {
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

    fn clock(&mut self) {
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

    fn sample(&self) -> f32 {
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
        assert_eq!(m.nt_target(0x2005), NtTarget::Ciram(0x005));
        assert_eq!(m.nt_target(0x2405), NtTarget::Ciram(0x405));
        assert_eq!(m.nt_target(0x2800), NtTarget::Cart);
        assert_eq!(m.ppu_read(0x2800), 12);
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
