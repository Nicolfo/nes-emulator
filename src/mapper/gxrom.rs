use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// GxROM (mapper 66): one register at $8000-$FFFF - bits 4-5 select a 32KB
/// PRG bank, bits 0-1 select an 8KB CHR bank. A header declaring no CHR ROM
/// gets 8KB of CHR RAM (the iNES convention), keeping such images from
/// crashing the bank math.
#[derive(Serialize, Deserialize)]
pub struct Gxrom {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    #[serde(default)]
    chr_is_ram: bool,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Gxrom {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Gxrom {
            prg,
            chr,
            chr_is_ram,
            prg_bank: 0,
            chr_bank: 0,
            mirroring,
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x2000;
        (self.chr_bank as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)
    }
}

impl Mapper for Gxrom {
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
            self.prg_bank = (val >> 4) & 3;
            self.chr_bank = val & 3;
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.chr[self.chr_offset(addr)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.chr_is_ram {
            let off = self.chr_offset(addr);
            self.chr[off] = val;
        }
    }

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
