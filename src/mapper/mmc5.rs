use super::{Mapper, Mirroring, NtTarget};
use serde::{Deserialize, Serialize};

/// MMC5 (mapper 5, Castlevania III, Just Breed, Uncharted Waters):
/// game-compatible core - all four PRG and CHR banking modes with banked
/// PRG RAM, per-quadrant nametable mapping (CIRAM/ExRAM/fill), the
/// scanline IRQ, the 8x16-sprite CHR set switch, the $5205 multiplier and
/// expansion audio (two APU-style pulses plus raw PCM).
///
/// Not emulated: vertical split mode ($5200-$5202).
#[derive(Serialize, Deserialize)]
pub struct Mmc5 {
    #[serde(skip)]
    prg: Vec<u8>,
    prg_ram: Vec<u8>,
    chr: Vec<u8>,
    #[serde(with = "crate::savestate::byte_array")]
    exram: [u8; 0x400],
    prg_mode: u8,
    chr_mode: u8,
    ram_protect1: u8,
    ram_protect2: u8,
    exram_mode: u8,
    nt_map: u8,
    fill_tile: u8,
    fill_attr: u8,
    // $5113-$5117 raw values; bit 7 of $5114-$5116 selects ROM vs RAM.
    prg_banks: [u8; 5],
    // 10-bit CHR banks: sprite set ($5120-$5127) and BG set ($5128-$512B).
    chr_sprite: [u16; 8],
    chr_bg: [u16; 4],
    chr_upper: u16,
    // In 8x8 sprite mode all fetches use the last-written register set.
    last_set_bg: bool,
    // ExGrafix (ExRAM mode 1): ExRAM byte latched at the tile's NT fetch;
    // bits 0-5 pick the tile's 4KB CHR bank, bits 6-7 its palette.
    ex_latch: u8,
    sprites_8x16: bool,
    rendering: bool,
    // Scanline detection: three consecutive identical NT fetches.
    last_nt_addr: u16,
    nt_streak: u8,
    in_frame: bool,
    scanline: u8,
    irq_compare: u8,
    irq_enabled: bool,
    irq_pending: bool,
    // Pattern fetches since the last detected scanline; 64..80 is the
    // sprite fetch window (PPU dots 257-320).
    fetch_count: u16,
    // CPU cycles with no PPU fetch; the real chip drops in-frame when the
    // PPU goes quiet (vblank or rendering disabled).
    ppu_idle: u8,
    multiplicand: u8,
    multiplier: u8,
    audio: Mmc5Audio,
}

impl Mmc5 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, _mirroring: Mirroring) -> Self {
        let chr = if chr.is_empty() { vec![0; 0x2000] } else { chr };
        Mmc5 {
            prg,
            prg_ram: vec![0; 0x10000],
            chr,
            exram: [0; 0x400],
            prg_mode: 3,
            chr_mode: 3,
            ram_protect1: 0,
            ram_protect2: 0,
            exram_mode: 0,
            nt_map: 0,
            fill_tile: 0,
            fill_attr: 0,
            prg_banks: [0, 0, 0, 0, 0xFF],
            chr_sprite: [0; 8],
            chr_bg: [0; 4],
            chr_upper: 0,
            last_set_bg: false,
            ex_latch: 0,
            sprites_8x16: false,
            rendering: false,
            last_nt_addr: 0,
            nt_streak: 0,
            in_frame: false,
            scanline: 0,
            irq_compare: 0,
            irq_enabled: false,
            irq_pending: false,
            fetch_count: 0,
            ppu_idle: 0,
            multiplicand: 0xFF,
            multiplier: 0xFF,
            audio: Mmc5Audio::new(),
        }
    }

    /// Resolve a CPU address ($6000-$FFFF) to (is_rom, byte offset).
    fn prg_resolve(&self, addr: u16) -> (bool, usize) {
        let rom_8k = |reg: u8| {
            let banks = self.prg.len() / 0x2000;
            (reg as usize & 0x7F) % banks * 0x2000 + (addr as usize & 0x1FFF)
        };
        let ram_8k = |reg: u8| {
            let banks = self.prg_ram.len() / 0x2000;
            (reg as usize & 0x0F) % banks * 0x2000 + (addr as usize & 0x1FFF)
        };
        let rom_16k = |reg: u8| {
            let banks = self.prg.len() / 0x4000;
            ((reg as usize & 0x7F) >> 1) % banks * 0x4000 + (addr as usize & 0x3FFF)
        };
        let ram_16k = |reg: u8| {
            let banks = (self.prg_ram.len() / 0x4000).max(1);
            ((reg as usize & 0x0F) >> 1) % banks * 0x4000 + (addr as usize & 0x3FFF)
        };
        let split = |reg: u8, wide: bool| {
            let rom = reg & 0x80 != 0;
            if wide {
                (rom, if rom { rom_16k(reg) } else { ram_16k(reg) })
            } else {
                (rom, if rom { rom_8k(reg) } else { ram_8k(reg) })
            }
        };
        if addr < 0x8000 {
            return (false, ram_8k(self.prg_banks[0]));
        }
        match self.prg_mode & 3 {
            0 => {
                let banks = self.prg.len() / 0x8000;
                let bank = (self.prg_banks[4] as usize & 0x7F) >> 2;
                (true, bank % banks * 0x8000 + (addr as usize & 0x7FFF))
            }
            1 => match addr {
                0x8000..=0xBFFF => split(self.prg_banks[2], true),
                _ => (true, rom_16k(self.prg_banks[4])),
            },
            2 => match addr {
                0x8000..=0xBFFF => split(self.prg_banks[2], true),
                0xC000..=0xDFFF => split(self.prg_banks[3], false),
                _ => (true, rom_8k(self.prg_banks[4])),
            },
            _ => match addr {
                0x8000..=0x9FFF => split(self.prg_banks[1], false),
                0xA000..=0xBFFF => split(self.prg_banks[2], false),
                0xC000..=0xDFFF => split(self.prg_banks[3], false),
                _ => (true, rom_8k(self.prg_banks[4])),
            },
        }
    }

    fn ram_writable(&self) -> bool {
        self.ram_protect1 & 3 == 2 && self.ram_protect2 & 3 == 1
    }

    fn chr_offset(&self, addr: u16, bg_set: bool) -> usize {
        let banks_of = |unit: usize| (self.chr.len() / unit).max(1);
        if bg_set {
            // The BG set covers 4KB, mirrored into both pattern tables
            // (mode 0 maps a full 8KB through $512B).
            match self.chr_mode & 3 {
                0 => {
                    let b = self.chr_bg[3] as usize % banks_of(0x2000);
                    b * 0x2000 + (addr as usize & 0x1FFF)
                }
                1 => {
                    let b = self.chr_bg[3] as usize % banks_of(0x1000);
                    b * 0x1000 + (addr as usize & 0x0FFF)
                }
                2 => {
                    let reg = self.chr_bg[((addr >> 11) as usize & 1) * 2 + 1];
                    reg as usize % banks_of(0x800) * 0x800 + (addr as usize & 0x7FF)
                }
                _ => {
                    let reg = self.chr_bg[(addr >> 10) as usize & 3];
                    reg as usize % banks_of(0x400) * 0x400 + (addr as usize & 0x3FF)
                }
            }
        } else {
            match self.chr_mode & 3 {
                0 => {
                    let b = self.chr_sprite[7] as usize % banks_of(0x2000);
                    b * 0x2000 + (addr as usize & 0x1FFF)
                }
                1 => {
                    let reg = self.chr_sprite[((addr >> 12) as usize & 1) * 4 + 3];
                    reg as usize % banks_of(0x1000) * 0x1000 + (addr as usize & 0x0FFF)
                }
                2 => {
                    let reg = self.chr_sprite[((addr >> 11) as usize & 3) * 2 + 1];
                    reg as usize % banks_of(0x800) * 0x800 + (addr as usize & 0x7FF)
                }
                _ => {
                    let reg = self.chr_sprite[(addr >> 10) as usize & 7];
                    reg as usize % banks_of(0x400) * 0x400 + (addr as usize & 0x3FF)
                }
            }
        }
    }

    /// Three identical consecutive NT fetches mark a new rendered scanline.
    fn scanline_detected(&mut self) {
        crate::trace_log!(
            "NES_MMC5_SL",
            "sl {} fc {}",
            self.scanline,
            self.fetch_count
        );
        self.fetch_count = 0;
        if !self.in_frame {
            self.in_frame = true;
            self.scanline = 0;
        } else {
            self.scanline = self.scanline.wrapping_add(1);
            if self.irq_compare != 0 && self.scanline == self.irq_compare {
                self.irq_pending = true;
                crate::trace_log!("NES_MMC5_LOG", "mmc5 irq pend at sl {}", self.scanline);
            }
        }
    }

    fn ppu_activity(&mut self) {
        self.ppu_idle = 0;
    }
}

impl Mapper for Mmc5 {
    crate::impl_mapper_savestate!();
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let (is_rom, off) = self.prg_resolve(addr);
        if is_rom {
            self.prg[off]
        } else {
            self.prg_ram[off]
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        #[cfg(feature = "trace")]
        if (0x5100..=0x5206).contains(&addr) && std::env::var("NES_MMC5_LOG").is_ok() {
            eprintln!(
                "mmc5 w {addr:04X} = {val:02X} (sl {})",
                if self.in_frame {
                    self.scanline as i16
                } else {
                    -1
                }
            );
        }
        match addr {
            0x5000..=0x5015 => self.audio.write(addr, val),
            0x5100 => self.prg_mode = val & 3,
            0x5101 => self.chr_mode = val & 3,
            0x5102 => self.ram_protect1 = val & 3,
            0x5103 => self.ram_protect2 = val & 3,
            0x5104 => self.exram_mode = val & 3,
            0x5105 => self.nt_map = val,
            0x5106 => self.fill_tile = val,
            0x5107 => self.fill_attr = val & 3,
            0x5113..=0x5117 => self.prg_banks[addr as usize - 0x5113] = val,
            0x5120..=0x5127 => {
                self.chr_sprite[addr as usize - 0x5120] = val as u16 | self.chr_upper;
                self.last_set_bg = false;
            }
            0x5128..=0x512B => {
                self.chr_bg[addr as usize - 0x5128] = val as u16 | self.chr_upper;
                self.last_set_bg = true;
            }
            0x5130 => self.chr_upper = ((val as u16) & 3) << 8,
            0x5203 => self.irq_compare = val,
            0x5204 => self.irq_enabled = val & 0x80 != 0,
            0x5205 => self.multiplicand = val,
            0x5206 => self.multiplier = val,
            0x5C00..=0x5FFF => {
                // ExRAM is CPU-writable except in read-only mode 3.
                if self.exram_mode != 3 {
                    self.exram[(addr & 0x3FF) as usize] = val;
                }
            }
            0x6000..=0x7FFF => {
                if self.ram_writable() {
                    let (_, off) = self.prg_resolve(addr);
                    self.prg_ram[off] = val;
                }
            }
            0x8000..=0xFFFF => {
                let (is_rom, off) = self.prg_resolve(addr);
                if !is_rom && self.ram_writable() {
                    self.prg_ram[off] = val;
                }
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.ppu_activity();
        if addr < 0x2000 {
            // Scanline detection needs three *consecutive PPU reads* of the
            // same NT address; a pattern fetch in between breaks the run
            // (the sprite window's paired garbage NT reads share one
            // address and would otherwise false-trigger).
            self.nt_streak = 0;
            let in_render = self.in_frame && self.rendering;
            let sprite_window = if in_render {
                self.fetch_count += 1;
                // Fetches 65-80 of a scanline are the sprite window.
                (65..=80).contains(&self.fetch_count)
            } else {
                false
            };
            if in_render && !sprite_window && self.exram_mode == 1 {
                // ExGrafix: the BG tile's 4KB bank comes from the ExRAM
                // byte latched at its NT fetch; $5130 supplies bits 6-7.
                let bank = (self.ex_latch as usize & 0x3F) | ((self.chr_upper as usize >> 8) << 6);
                let banks = (self.chr.len() / 0x1000).max(1);
                return self.chr[bank % banks * 0x1000 + (addr as usize & 0xFFF)];
            }
            let bg_set = if in_render && self.sprites_8x16 {
                !sprite_window
            } else {
                self.last_set_bg
            };
            self.chr[self.chr_offset(addr, bg_set)]
        } else {
            // ExGrafix attribute fetch: palette bits of the latched ExRAM
            // byte, replicated to every quadrant of the attribute byte.
            if self.exram_mode == 1 && self.in_frame && self.rendering && addr & 0x3FF >= 0x3C0 {
                return (self.ex_latch >> 6) * 0x55;
            }
            // Nametable access routed here by nt_target: ExRAM or fill mode.
            let q = (self.nt_map >> (((addr >> 10) & 3) * 2)) & 3;
            if q == 2 {
                if self.exram_mode < 2 {
                    self.exram[(addr & 0x3FF) as usize]
                } else {
                    0
                }
            } else if addr & 0x3FF >= 0x3C0 {
                self.fill_attr * 0x55 // attribute byte replicated to all quads
            } else {
                self.fill_tile
            }
        }
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        self.ppu_activity();
        if addr < 0x2000 {
            self.nt_streak = 0;
        } else {
            let q = (self.nt_map >> (((addr >> 10) & 3) * 2)) & 3;
            if q == 2 && self.exram_mode < 2 {
                self.exram[(addr & 0x3FF) as usize] = val;
            }
            // Fill mode ignores writes; CHR is ROM.
        }
    }

    fn mirroring(&self) -> Mirroring {
        // Unused: nt_target below overrides nametable routing.
        Mirroring::Vertical
    }

    fn nt_target(&mut self, addr: u16) -> NtTarget {
        self.ppu_activity();
        if addr == self.last_nt_addr {
            self.nt_streak += 1;
            if self.nt_streak == 3 {
                self.scanline_detected();
            }
        } else {
            self.last_nt_addr = addr;
            self.nt_streak = 1;
        }
        if self.exram_mode == 1 && self.in_frame && self.rendering {
            if addr & 0x3FF >= 0x3C0 {
                // ExGrafix substitutes the attribute fetch (see ppu_read).
                return NtTarget::Cart;
            }
            self.ex_latch = self.exram[(addr & 0x3FF) as usize];
        }
        let q = (self.nt_map >> (((addr >> 10) & 3) * 2)) & 3;
        match q {
            0 => NtTarget::Ciram(addr & 0x3FF),
            1 => NtTarget::Ciram(0x400 | (addr & 0x3FF)),
            _ => NtTarget::Cart,
        }
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        let (_, off) = self.prg_resolve(addr);
        Some(self.prg_ram[off])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.prg_ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.prg_ram)
    }

    fn cpu_reg_read(&mut self, addr: u16) -> Option<u8> {
        match addr {
            0x5015 => Some(self.audio.status()),
            0x5204 => {
                let v = (self.irq_pending as u8) << 7 | (self.in_frame as u8) << 6;
                self.irq_pending = false;
                crate::trace_log!(
                    "NES_MMC5_LOG",
                    "mmc5 r 5204 = {v:02X} (sl {})",
                    if self.in_frame {
                        self.scanline as i16
                    } else {
                        -1
                    }
                );
                Some(v)
            }
            0x5205 => Some((self.multiplicand as u16 * self.multiplier as u16) as u8),
            0x5206 => Some(((self.multiplicand as u16 * self.multiplier as u16) >> 8) as u8),
            0x5C00..=0x5FFF if self.exram_mode >= 2 => Some(self.exram[(addr & 0x3FF) as usize]),
            _ => None,
        }
    }

    fn irq(&self) -> bool {
        self.irq_pending && self.irq_enabled
    }

    fn cpu_clock(&mut self) {
        // The PPU fetches continuously while rendering; ~3 idle CPU cycles
        // mean vblank or rendering off, which ends the in-frame state.
        if self.ppu_idle < 3 {
            self.ppu_idle += 1;
        } else if self.in_frame {
            self.in_frame = false;
            self.nt_streak = 0;
            self.fetch_count = 0;
        }
        self.audio.clock();
    }

    fn audio_sample(&self) -> f32 {
        self.audio.sample()
    }

    fn cpu_bus_write(&mut self, addr: u16, val: u8) {
        #[cfg(feature = "trace")]
        if matches!(addr & 7, 0 | 1 | 5 | 6) && std::env::var("NES_MMC5_LOG").is_ok() {
            eprintln!(
                "mmc5 snoop {:04X} = {val:02X} (sl {})",
                0x2000 + (addr & 7),
                if self.in_frame {
                    self.scanline as i16
                } else {
                    -1
                }
            );
        }
        match addr & 7 {
            0 => self.sprites_8x16 = val & 0x20 != 0,
            1 => {
                self.rendering = val & 0x18 != 0;
                if !self.rendering {
                    self.in_frame = false;
                    self.nt_streak = 0;
                    self.fetch_count = 0;
                }
            }
            _ => {}
        }
    }
}

/// MMC5 audio: two APU-style pulse channels (envelope and length counter,
/// no sweep) sequenced by an internal ~240 Hz divider, plus raw PCM.
#[derive(Serialize, Deserialize)]
struct Mmc5Audio {
    pulses: [Pulse; 2],
    pcm_level: u8,
    cycle: u32,
    frame_step: u8,
}

#[derive(Default, Serialize, Deserialize)]
struct Pulse {
    duty: u8,
    halt: bool,
    const_vol: bool,
    vol: u8,
    period: u16,
    timer: u16,
    step: u8,
    length: u8,
    enabled: bool,
    env_start: bool,
    env_divider: u8,
    env_decay: u8,
}

const DUTY: [u8; 4] = [0b0100_0000, 0b0110_0000, 0b0111_1000, 0b1001_1111];

const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

impl Pulse {
    fn write(&mut self, reg: u16, val: u8) {
        match reg {
            0 => {
                self.duty = val >> 6;
                self.halt = val & 0x20 != 0;
                self.const_vol = val & 0x10 != 0;
                self.vol = val & 0x0F;
            }
            2 => self.period = (self.period & 0x700) | val as u16,
            3 => {
                self.period = (self.period & 0x0FF) | ((val as u16 & 7) << 8);
                if self.enabled {
                    self.length = LENGTH_TABLE[(val >> 3) as usize];
                }
                self.step = 0;
                self.env_start = true;
            }
            _ => {}
        }
    }

    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.period;
            self.step = (self.step + 1) & 7;
        } else {
            self.timer -= 1;
        }
    }

    fn clock_quarter(&mut self) {
        if self.env_start {
            self.env_start = false;
            self.env_decay = 15;
            self.env_divider = self.vol;
        } else if self.env_divider == 0 {
            self.env_divider = self.vol;
            if self.env_decay > 0 {
                self.env_decay -= 1;
            } else if self.halt {
                self.env_decay = 15;
            }
        } else {
            self.env_divider -= 1;
        }
    }

    fn clock_half(&mut self) {
        if !self.halt && self.length > 0 {
            self.length -= 1;
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled || self.length == 0 || DUTY[self.duty as usize] & (0x80 >> self.step) == 0
        {
            0
        } else if self.const_vol {
            self.vol
        } else {
            self.env_decay
        }
    }
}

impl Mmc5Audio {
    fn new() -> Self {
        Mmc5Audio {
            pulses: [Pulse::default(), Pulse::default()],
            pcm_level: 0,
            cycle: 0,
            frame_step: 0,
        }
    }

    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x5000..=0x5003 => self.pulses[0].write(addr & 3, val),
            0x5004..=0x5007 => self.pulses[1].write(addr & 3, val),
            // $5010 selects PCM read mode / IRQ; only write mode ($5011)
            // is emulated.
            0x5011 => {
                if val != 0 {
                    self.pcm_level = val;
                }
            }
            0x5015 => {
                for (i, p) in self.pulses.iter_mut().enumerate() {
                    p.enabled = val & (1 << i) != 0;
                    if !p.enabled {
                        p.length = 0;
                    }
                }
            }
            _ => {}
        }
    }

    fn status(&self) -> u8 {
        (self.pulses[0].length > 0) as u8 | ((self.pulses[1].length > 0) as u8) << 1
    }

    fn clock(&mut self) {
        self.cycle += 1;
        if self.cycle & 1 == 0 {
            self.pulses[0].clock_timer();
            self.pulses[1].clock_timer();
        }
        // ~240 Hz envelope clock, ~120 Hz length clock.
        if self.cycle.is_multiple_of(7457) {
            self.frame_step = (self.frame_step + 1) & 3;
            for p in &mut self.pulses {
                p.clock_quarter();
                if self.frame_step & 1 == 0 {
                    p.clock_half();
                }
            }
        }
    }

    fn sample(&self) -> f32 {
        let p = (self.pulses[0].output() + self.pulses[1].output()) as f32;
        let pulse_out = if p > 0.0 {
            95.52 / (8128.0 / p + 100.0)
        } else {
            0.0
        };
        pulse_out + self.pcm_level as f32 * 0.002
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mmc5() -> Mmc5 {
        // 16 x 8KB PRG (128KB), 32 x 1KB CHR; byte = bank index.
        let prg: Vec<u8> = (0..16 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..32 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Mmc5::new(prg, chr, Mirroring::Horizontal)
    }

    #[test]
    fn multiplier() {
        let mut m = mmc5();
        m.cpu_write(0x5205, 0x34);
        m.cpu_write(0x5206, 0x56);
        let prod = 0x34u16 * 0x56;
        assert_eq!(m.cpu_reg_read(0x5205), Some(prod as u8));
        assert_eq!(m.cpu_reg_read(0x5206), Some((prod >> 8) as u8));
    }

    #[test]
    fn prg_mode_3_with_ram() {
        let mut m = mmc5();
        m.cpu_write(0x5100, 3);
        m.cpu_write(0x5114, 0x82); // ROM bank 2 at $8000
        m.cpu_write(0x5115, 0x01); // RAM bank 1 at $A000
        m.cpu_write(0x5116, 0x85); // ROM bank 5 at $C000
        m.cpu_write(0x5117, 0x0F); // $E000 is always ROM
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xC000), 5);
        assert_eq!(m.cpu_read(0xE000), 15);
        // RAM mapped at $A000: write with protect off, read back.
        m.cpu_write(0x5102, 2);
        m.cpu_write(0x5103, 1);
        m.cpu_write(0xA000, 0xCD);
        assert_eq!(m.cpu_read(0xA000), 0xCD);
        // Protect on: writes dropped.
        m.cpu_write(0x5103, 0);
        m.cpu_write(0xA000, 0x11);
        assert_eq!(m.cpu_read(0xA000), 0xCD);
    }

    #[test]
    fn prg_modes_0_1_2() {
        let mut m = mmc5();
        m.cpu_write(0x5117, 0x8F);
        m.cpu_write(0x5100, 0); // 32KB: bank $8F>>2 = 3 -> 8KB banks 12-15
        assert_eq!(m.cpu_read(0x8000), 12);
        assert_eq!(m.cpu_read(0xE000), 15);
        m.cpu_write(0x5100, 1); // 16KB halves
        m.cpu_write(0x5115, 0x84); // 16KB bank 2 -> 8KB banks 4-5
        assert_eq!(m.cpu_read(0x8000), 4);
        assert_eq!(m.cpu_read(0xA000), 5);
        assert_eq!(m.cpu_read(0xC000), 14); // $5117=$8F -> 16KB bank 7
        m.cpu_write(0x5100, 2);
        m.cpu_write(0x5116, 0x86);
        assert_eq!(m.cpu_read(0xC000), 6);
        assert_eq!(m.cpu_read(0xE000), 15);
    }

    #[test]
    fn banked_prg_ram_at_6000() {
        let mut m = mmc5();
        m.cpu_write(0x5102, 2);
        m.cpu_write(0x5103, 1);
        m.cpu_write(0x5113, 0);
        m.cpu_write(0x6000, 0xAA);
        m.cpu_write(0x5113, 3);
        m.cpu_write(0x6000, 0xBB);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xBB));
        m.cpu_write(0x5113, 0);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAA));
    }

    #[test]
    fn chr_modes() {
        let mut m = mmc5();
        m.cpu_write(0x5101, 3); // 1KB
        m.cpu_write(0x5127, 9);
        assert_eq!(m.ppu_read(0x1C00), 9);
        m.cpu_write(0x5101, 1); // 4KB: sprite regs 3/7
        m.cpu_write(0x5123, 4); // 4KB bank 4 -> 1KB banks 16-19
        assert_eq!(m.ppu_read(0x0400), 17);
        m.cpu_write(0x5101, 0); // 8KB: reg 7
        m.cpu_write(0x5127, 2); // 8KB bank 2 -> 1KB banks 16-23
        assert_eq!(m.ppu_read(0x0000), 16);
    }

    #[test]
    fn bg_set_used_when_written_last() {
        let mut m = mmc5();
        m.cpu_write(0x5101, 3);
        m.cpu_write(0x5120, 5); // sprite set, bank 5 at $0000
        assert_eq!(m.ppu_read(0x0000), 5);
        m.cpu_write(0x5128, 7); // BG set written last
        assert_eq!(m.ppu_read(0x0000), 7);
        // BG set mirrors its 4KB across both pattern tables.
        m.cpu_write(0x512B, 9);
        assert_eq!(m.ppu_read(0x1C00), 9);
    }

    #[test]
    fn nametable_quadrants_exram_and_fill() {
        let mut m = mmc5();
        // NT0 CIRAM0, NT1 CIRAM1, NT2 ExRAM, NT3 fill.
        m.cpu_write(0x5105, 0b11_10_01_00);
        assert_eq!(m.nt_target(0x2010), NtTarget::Ciram(0x010));
        assert_eq!(m.nt_target(0x2410), NtTarget::Ciram(0x410));
        assert_eq!(m.nt_target(0x2810), NtTarget::Cart);
        assert_eq!(m.nt_target(0x2C10), NtTarget::Cart);
        // ExRAM nametable readable/writable via PPU in modes 0/1.
        m.ppu_write(0x2810, 0x42);
        assert_eq!(m.ppu_read(0x2810), 0x42);
        // Fill mode returns the fill tile / attribute.
        m.cpu_write(0x5106, 0x7E);
        m.cpu_write(0x5107, 0x02);
        assert_eq!(m.ppu_read(0x2C10), 0x7E);
        assert_eq!(m.ppu_read(0x2FD0), 0x02 * 0x55);
    }

    #[test]
    fn exram_cpu_access_modes() {
        let mut m = mmc5();
        m.cpu_write(0x5104, 2); // CPU RAM mode
        m.cpu_write(0x5C05, 0x99);
        assert_eq!(m.cpu_reg_read(0x5C05), Some(0x99));
        m.cpu_write(0x5104, 3); // read-only
        m.cpu_write(0x5C05, 0x11);
        assert_eq!(m.cpu_reg_read(0x5C05), Some(0x99));
        m.cpu_write(0x5104, 0); // NT mode: CPU reads are open bus
        assert_eq!(m.cpu_reg_read(0x5C05), None);
    }

    #[test]
    fn scanline_irq_from_nt_fetch_pattern() {
        let mut m = mmc5();
        m.cpu_write(0x5203, 2); // IRQ at scanline 2
        m.cpu_write(0x5204, 0x80); // enable
        // Simulate scanlines: 3 identical NT fetches each, with ordinary
        // varied fetches in between.
        for line in 0u16..4 {
            for tile in 0..8 {
                m.nt_target(0x2000 + line * 32 + tile);
            }
            let probe = 0x23A0 + line;
            for _ in 0..3 {
                m.nt_target(probe);
            }
            match line {
                0 | 1 => assert!(!m.irq(), "IRQ too early at line {line}"),
                _ => assert!(m.irq(), "IRQ expected by line {line}"),
            }
        }
        // $5204 read reports and acknowledges.
        let status = m.cpu_reg_read(0x5204).unwrap();
        assert_eq!(status & 0xC0, 0xC0); // pending + in-frame
        assert!(!m.irq());
        // PPU going idle (vblank) clears in-frame.
        for _ in 0..10 {
            m.cpu_clock();
        }
        assert_eq!(m.cpu_reg_read(0x5204).unwrap() & 0x40, 0);
    }

    #[test]
    fn exgrafix_per_tile_bank_and_attribute() {
        let mut m = mmc5(); // 32 x 1KB CHR = 8 x 4KB banks
        m.cpu_write(0x5104, 1); // ExGrafix
        m.cpu_bus_write(0x2001, 0x1E); // rendering on
        // ExRAM byte for tile 5: 4KB bank 3, palette 2.
        m.cpu_write(0x5C05, 0x83);
        // Establish in-frame (scanline detection), then fetch tile 5's NT.
        for _ in 0..3 {
            m.nt_target(0x2300);
        }
        m.nt_target(0x2005);
        // BG pattern fetches use 4KB bank 3 = 1KB banks 12-15.
        assert_eq!(m.ppu_read(0x0000), 12);
        assert_eq!(m.ppu_read(0x0FFF), 15);
        // The attribute fetch is substituted from the latch.
        assert_eq!(m.nt_target(0x23C5), NtTarget::Cart);
        assert_eq!(m.ppu_read(0x23C5), 0xAA); // palette 2 replicated
        // Outside rendering, mode 1 falls back to normal banking.
        m.cpu_bus_write(0x2001, 0x00);
        m.cpu_write(0x5120, 4);
        assert_eq!(m.ppu_read(0x0000), 4);
    }

    #[test]
    fn sprite_window_garbage_nt_reads_do_not_false_trigger() {
        let mut m = mmc5();
        m.cpu_write(0x5203, 1); // IRQ at scanline 1
        m.cpu_write(0x5204, 0x80);
        // Real scanline detection: three identical NT reads in a row.
        for _ in 0..3 {
            m.nt_target(0x2000);
        }
        // Sprite window: 8 units of paired garbage NT reads at a single
        // address, each pair followed by the two pattern-plane fetches.
        for _ in 0..8 {
            m.nt_target(0x20A5);
            m.nt_target(0x20A5);
            m.ppu_read(0x0000);
            m.ppu_read(0x0008);
        }
        assert!(!m.irq(), "garbage NT pairs must not clock the counter");
        // The next real detection clocks scanline 1 and fires.
        for _ in 0..3 {
            m.nt_target(0x2042);
        }
        assert!(m.irq());
    }

    #[test]
    fn pulse_audio_output() {
        let mut m = mmc5();
        m.cpu_write(0x5015, 0x01);
        m.cpu_write(0x5000, 0xBF); // duty 2, halt, constant volume 15
        m.cpu_write(0x5002, 0x20);
        m.cpu_write(0x5003, 0x00);
        let mut peak = 0.0f32;
        for _ in 0..2000 {
            m.cpu_clock();
            peak = peak.max(m.audio_sample());
        }
        assert!(peak > 0.05, "expected MMC5 pulse output, got {peak}");
        let before = m.audio_sample();
        m.cpu_write(0x5011, 0x40);
        let delta = m.audio_sample() - before;
        assert!(
            (delta - 0x40 as f32 * 0.002).abs() < 1e-6,
            "PCM delta {delta}"
        );
    }
}
