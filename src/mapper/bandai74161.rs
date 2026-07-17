use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Bandai / Jaleco 74161-style discrete board (mappers 70 and 152). A single
/// write port at $8000-$FFFF latches one byte:
///
/// ```text
/// 7  bit  0
/// M PPP CCCC
/// | |   |
/// | |   +--- bits 0-3: 8KB CHR bank (switchable)
/// | +------- bits 4-6: 16KB PRG bank at $8000 (switchable)
/// +--------- bit 7: one-screen mirroring select (mapper 152 only)
/// ```
/// PRG: $8000-$BFFF switchable, $C000-$FFFF fixed to the last 16KB (UxROM-like).
///
/// Mapper 70 ignores bit 7 and keeps the header-supplied mirroring; mapper 152
/// uses bit 7 to pick single-screen page A (0) or B (1). Games: Kamen Rider
/// Club, Family Trainer (70); Arkanoid II, Saint Seiya, Pocket Zaurus (152).
#[derive(Serialize, Deserialize)]
pub struct Bandai74161 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
    /// Mapper 152: bit 7 drives one-screen mirroring.
    mirror_control: bool,
    four_screen: bool,
}

impl Bandai74161 {
    pub fn new(mapper_id: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        let four_screen = mirroring == Mirroring::FourScreen;
        Bandai74161 {
            prg,
            chr,
            chr_is_ram,
            prg_bank: 0,
            chr_bank: 0,
            mirroring,
            mirror_control: mapper_id == 152,
            four_screen,
        }
    }
}

impl Mapper for Bandai74161 {
    crate::impl_mapper_savestate!(chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        let banks = self.prg.len() / 0x4000;
        match addr {
            0x8000..=0xBFFF => {
                self.prg[(self.prg_bank as usize % banks) * 0x4000 + (addr as usize & 0x3FFF)]
            }
            0xC000..=0xFFFF => self.prg[(banks - 1) * 0x4000 + (addr as usize & 0x3FFF)],
            _ => 0,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            self.prg_bank = (val >> 4) & 0x07;
            self.chr_bank = val & 0x0F;
            if self.mirror_control && !self.four_screen {
                self.mirroring = if val & 0x80 != 0 {
                    Mirroring::SingleScreenHi
                } else {
                    Mirroring::SingleScreenLo
                };
            }
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let banks = self.chr.len() / 0x2000;
        self.chr[(self.chr_bank as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.chr_is_ram {
            self.chr[(addr as usize) & 0x1FFF] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(mapper_id: u8) -> Bandai74161 {
        // 8 PRG 16KB banks, 8 CHR 8KB banks; byte = bank index.
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        Bandai74161::new(mapper_id, prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_switch_fixed_last() {
        let mut m = board(70);
        m.cpu_write(0x8000, 0x30); // PRG bank = bits 4-6 = 3
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xC000), 7); // last fixed
    }

    #[test]
    fn chr_switch() {
        let mut m = board(70);
        m.cpu_write(0x8000, 0x05); // CHR bank = bits 0-3 = 5
        assert_eq!(m.ppu_read(0x0000), 5);
    }

    #[test]
    fn mapper70_ignores_bit7() {
        let mut m = board(70);
        m.cpu_write(0x8000, 0x80);
        assert_eq!(m.mirroring(), Mirroring::Vertical); // header mirroring kept
    }

    #[test]
    fn mapper152_one_screen_select() {
        let mut m = board(152);
        m.cpu_write(0x8000, 0x00);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        m.cpu_write(0x8000, 0x80);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
    }

    #[test]
    fn chr_ram_fallback() {
        let prg: Vec<u8> = (0..2 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let mut m = Bandai74161::new(70, prg, vec![], Mirroring::Vertical);
        m.ppu_write(0x1234, 0xAB);
        assert_eq!(m.ppu_read(0x1234), 0xAB);
    }
}
