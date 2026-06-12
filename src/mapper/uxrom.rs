use super::{Mapper, Mirroring};

/// UxROM (mapper 2): 16KB switchable PRG at $8000, last 16KB fixed at $C000,
/// 8KB CHR (usually RAM). Real boards have bus conflicts (written value
/// ANDed with ROM byte); not emulated.
pub struct Uxrom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_bank: u8,
    mirroring: Mirroring,
}

impl Uxrom {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Uxrom {
            prg,
            chr,
            chr_is_ram,
            prg_bank: 0,
            mirroring,
        }
    }
}

impl Mapper for Uxrom {
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
            self.prg_bank = val;
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

    fn uxrom() -> Uxrom {
        // 8 PRG banks (128KB), CHR RAM; each PRG byte = its bank index.
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        Uxrom::new(prg, vec![], Mirroring::Vertical)
    }

    #[test]
    fn switchable_8000_fixed_c000() {
        let mut m = uxrom();
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xC000), 7);
        m.cpu_write(0x8000, 2);
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xC000), 7);
    }

    #[test]
    fn bank_wraps_modulo() {
        let mut m = uxrom();
        m.cpu_write(0x8000, 9);
        assert_eq!(m.cpu_read(0x8000), 1);
    }

    #[test]
    fn chr_ram_rw() {
        let mut m = uxrom();
        m.ppu_write(0x1234, 0xAB);
        assert_eq!(m.ppu_read(0x1234), 0xAB);
    }
}
