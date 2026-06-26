use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// GxROM (mapper 66): one register at $8000-$FFFF - bits 4-5 select a 32KB
/// PRG bank, bits 0-1 select an 8KB CHR bank.
#[derive(Serialize, Deserialize)]
pub struct Gxrom {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Gxrom {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        Gxrom {
            prg,
            chr,
            prg_bank: 0,
            chr_bank: 0,
            mirroring,
        }
    }
}

impl Mapper for Gxrom {
    crate::impl_mapper_savestate!();
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            let banks = self.prg.len() / 0x8000;
            self.prg[(self.prg_bank as usize % banks) * 0x8000 + (addr as usize & 0x7FFF)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            self.prg_bank = (val >> 4) & 3;
            self.chr_bank = val & 3;
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let banks = self.chr.len() / 0x2000;
        self.chr[(self.chr_bank as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)]
    }

    fn ppu_write(&mut self, _addr: u16, _val: u8) {}

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gxrom() -> Gxrom {
        // 4 32KB PRG banks, 4 8KB CHR banks; each byte = its bank index.
        let prg: Vec<u8> = (0..4 * 0x8000).map(|i| (i / 0x8000) as u8).collect();
        let chr: Vec<u8> = (0..4 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        Gxrom::new(prg, chr, Mirroring::Horizontal)
    }

    #[test]
    fn combined_reg_switches_prg_and_chr() {
        let mut m = gxrom();
        m.cpu_write(0x8000, 0x21); // PRG bank 2, CHR bank 1
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xFFFF), 2);
        assert_eq!(m.ppu_read(0x0000), 1);
    }
}
