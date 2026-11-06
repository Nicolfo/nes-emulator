use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Irem LROG017 (mapper 77), Napoleon Senki. One latch at $8000-$FFFF:
/// `[CCCC PPPP]` selects a 32KB PRG bank and a 2KB CHR-ROM bank at
/// $0000-$07FF. $0800-$1FFF is 6KB of CHR RAM, and the board's remaining
/// 2KB of RAM backs the extra nametables: always four-screen.
#[derive(Serialize, Deserialize)]
pub struct IremLrog017 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_ram: Vec<u8>,
    prg_bank: u8,
    chr_bank: u8,
}

impl IremLrog017 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>) -> Self {
        // A dump without CHR ROM still needs something behind $0000-$07FF.
        let chr = if chr.is_empty() { vec![0; 0x800] } else { chr };
        IremLrog017 {
            prg,
            chr,
            chr_ram: vec![0; 0x1800],
            prg_bank: 0,
            chr_bank: 0,
        }
    }
}

impl Mapper for IremLrog017 {
    crate::impl_mapper_savestate!(chr, chr_ram);

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let off = self.prg_bank as usize * 0x8000 + (addr as usize & 0x7FFF);
        self.prg[off % self.prg.len()]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            self.prg_bank = val & 0x0F;
            self.chr_bank = val >> 4;
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let a = addr as usize & 0x1FFF;
        if a < 0x800 {
            let off = self.chr_bank as usize * 0x800 + a;
            self.chr[off % self.chr.len()]
        } else {
            self.chr_ram[a - 0x800]
        }
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        let a = addr as usize & 0x1FFF;
        if a >= 0x800 {
            self.chr_ram[a - 0x800] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        Mirroring::FourScreen
    }
}

/// Irem TAM-S1 (mapper 97), Kaiketsu Yanchamaru. $8000-$BFFF is fixed to
/// the last 16KB bank and $C000-$FFFF is switchable - the reverse of UxROM.
/// The latch at $8000-$FFFF is `[MM.P PPPP]`: `P` the bank, `M` mirroring
/// (0 = one-screen A, 1 = horizontal, 2 = vertical, 3 = one-screen B).
#[derive(Serialize, Deserialize)]
pub struct IremTamS1 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_bank: u8,
    mirroring: Mirroring,
    four_screen: bool,
}

impl IremTamS1 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        IremTamS1 {
            prg,
            chr,
            chr_is_ram,
            prg_bank: 0,
            four_screen: mirroring == Mirroring::FourScreen,
            mirroring,
        }
    }
}

impl Mapper for IremTamS1 {
    crate::impl_mapper_savestate!(chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        let banks = self.prg.len() / 0x4000;
        let bank = match addr {
            0x8000..=0xBFFF => banks - 1,
            0xC000..=0xFFFF => self.prg_bank as usize % banks,
            _ => return 0,
        };
        self.prg[bank * 0x4000 + (addr as usize & 0x3FFF)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr < 0x8000 {
            return;
        }
        self.prg_bank = val & 0x1F;
        if !self.four_screen {
            self.mirroring = match val >> 6 {
                0 => Mirroring::SingleScreenLo,
                1 => Mirroring::Horizontal,
                2 => Mirroring::Vertical,
                _ => Mirroring::SingleScreenHi,
            };
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.chr[(addr as usize & 0x1FFF) % self.chr.len()]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.chr_is_ram {
            let len = self.chr.len();
            self.chr[(addr as usize & 0x1FFF) % len] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrog017_banks_rom_and_ram_halves() {
        let prg: Vec<u8> = (0..4 * 0x8000).map(|i| (i / 0x8000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x800).map(|i| (i / 0x800) as u8 + 0x10).collect();
        let mut m = IremLrog017::new(prg, chr);
        m.cpu_write(0x8000, 0x32);
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.ppu_read(0x0000), 0x13);
        m.ppu_write(0x0000, 0xAA); // ROM: ignored
        assert_eq!(m.ppu_read(0x0000), 0x13);
        m.ppu_write(0x1FFF, 0x55); // RAM
        assert_eq!(m.ppu_read(0x1FFF), 0x55);
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
    }

    #[test]
    fn tam_s1_reversed_windows_and_mirroring() {
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let mut m = IremTamS1::new(prg, vec![], Mirroring::Vertical);
        m.cpu_write(0x8000, 0x80 | 3);
        assert_eq!(m.cpu_read(0x8000), 7);
        assert_eq!(m.cpu_read(0xC000), 3);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0xFFFF, 0x40);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0xFFFF, 0xC0);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
    }
}
