use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Camerica/Codemasters (mapper 71): a UxROM-like board with a 16KB switchable
/// PRG bank at $8000-$BFFF, the last 16KB fixed at $C000-$FFFF, and 8KB CHR
/// RAM. The bank register sits at $C000-$FFFF. A few BF9097 titles (Fire Hawk)
/// also drive single-screen mirroring from $9000-$9FFF bit 4.
#[derive(Clone, Serialize, Deserialize)]
pub struct Codemasters {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_bank: u8,
    mirroring: Mirroring,
}

impl Codemasters {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Codemasters {
            prg,
            chr,
            chr_is_ram,
            prg_bank: 0,
            mirroring,
        }
    }
}

impl Mapper for Codemasters {
    crate::impl_mapper_savestate!(chr_is_ram = chr_is_ram);
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
        match addr {
            // Fire Hawk single-screen mirroring (BF9097).
            0x9000..=0x9FFF => {
                if self.mirroring != Mirroring::FourScreen {
                    self.mirroring = if val & 0x10 != 0 {
                        Mirroring::SingleScreenHi
                    } else {
                        Mirroring::SingleScreenLo
                    };
                }
            }
            0xC000..=0xFFFF => self.prg_bank = val,
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.chr[(addr as usize) & 0x1FFF]
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

    fn codemasters() -> Codemasters {
        // 8 PRG banks (128KB), CHR RAM; each PRG byte = its 16KB bank index.
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        Codemasters::new(prg, vec![], Mirroring::Vertical)
    }

    #[test]
    fn switchable_8000_fixed_c000() {
        let mut m = codemasters();
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xC000), 7);
        m.cpu_write(0xC000, 3); // bank register at $C000-$FFFF
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xC000), 7); // still fixed last
    }

    #[test]
    fn fire_hawk_mirroring() {
        let mut m = codemasters();
        m.cpu_write(0x9000, 0x10);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
        m.cpu_write(0x9000, 0x00);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
    }

    #[test]
    fn chr_ram_rw() {
        let mut m = codemasters();
        m.ppu_write(0x0123, 0x42);
        assert_eq!(m.ppu_read(0x0123), 0x42);
    }
}
