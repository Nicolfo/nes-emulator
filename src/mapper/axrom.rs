use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// AxROM (mapper 7): one register at $8000-$FFFF - bits 0-2 select a 32KB
/// PRG bank, bit 4 selects the nametable (single-screen mirroring). CHR is
/// 8KB RAM. Header mirroring is ignored; the register controls it.
#[derive(Serialize, Deserialize)]
pub struct Axrom {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_bank: u8,
    mirroring: Mirroring,
}

impl Axrom {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Axrom {
            prg,
            chr,
            chr_is_ram,
            prg_bank: 0,
            mirroring: Mirroring::SingleScreenLo,
        }
    }
}

impl Mapper for Axrom {
    crate::impl_mapper_savestate!(chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            // A 16KB image can't fill this board's 32KB window; mirror it
            // (the loader guarantees at least one whole 16KB bank).
            if self.prg.len() < 0x8000 {
                return self.prg[addr as usize & (self.prg.len() - 1)];
            }
            let banks = self.prg.len() / 0x8000;
            self.prg[(self.prg_bank as usize % banks) * 0x8000 + (addr as usize & 0x7FFF)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            self.prg_bank = val & 7;
            self.mirroring = if val & 0x10 != 0 {
                Mirroring::SingleScreenHi
            } else {
                Mirroring::SingleScreenLo
            };
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

    fn axrom() -> Axrom {
        // 8 32KB PRG banks (256KB, like Battletoads), CHR RAM; each PRG
        // byte = its bank index.
        let prg: Vec<u8> = (0..8 * 0x8000).map(|i| (i / 0x8000) as u8).collect();
        Axrom::new(prg, vec![])
    }

    #[test]
    fn prg_32k_switch() {
        let mut m = axrom();
        assert_eq!(m.cpu_read(0x8000), 0);
        m.cpu_write(0x8000, 1);
        assert_eq!(m.cpu_read(0x8000), 1);
        assert_eq!(m.cpu_read(0xFFFF), 1);
    }

    #[test]
    fn nametable_select() {
        let mut m = axrom();
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        m.cpu_write(0x8000, 0x10);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
        m.cpu_write(0x8000, 0x00);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
    }

    #[test]
    fn chr_ram_rw() {
        let mut m = axrom();
        m.ppu_write(0x0042, 0x99);
        assert_eq!(m.ppu_read(0x0042), 0x99);
    }
}
