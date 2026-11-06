use super::discrete::{bank_index, chr_or_ram};
use super::{Mapper, Mirroring, NtTarget};
use serde::{Deserialize, Serialize};

/// GTROM / Cheapocabra (mapper 111), Membler Industries' homebrew board.
/// Register at $5000-$5FFF and $7000-$7FFF: bits 0-3 select a 32KB PRG
/// bank, bit 4 one of two 8KB CHR RAM banks, bit 5 one of two 4KB pages of
/// the board's 8KB nametable RAM (four-screen, all of $2000-$2FFF), bits 6-7
/// drive LEDs. The self-flashing PRG is not emulated (writes are ignored).
#[derive(Serialize, Deserialize)]
pub struct Gtrom {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    nt_ram: Vec<u8>,
    reg: u8,
}

impl Gtrom {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>) -> Self {
        let (chr, chr_is_ram) = chr_or_ram(chr, 0x4000);
        Gtrom {
            prg,
            chr,
            chr_is_ram,
            nt_ram: vec![0; 0x2000],
            reg: 0,
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        bank_index(
            self.chr.len(),
            0x2000,
            ((self.reg >> 4) & 1) as usize,
            addr as usize,
        )
    }

    fn nt_offset(&self, addr: u16) -> usize {
        ((self.reg as usize >> 5) & 1) * 0x1000 + (addr as usize & 0x0FFF)
    }
}

impl Mapper for Gtrom {
    crate::impl_mapper_savestate!(chr, nt_ram);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            let i = bank_index(
                self.prg.len(),
                0x8000,
                (self.reg & 0x0F) as usize,
                addr as usize,
            );
            self.prg[i]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if matches!(addr, 0x5000..=0x5FFF | 0x7000..=0x7FFF) {
            self.reg = val;
        }
    }

    fn nt_target(&mut self, _addr: u16) -> NtTarget {
        NtTarget::Cart
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x2000 {
            self.nt_ram[self.nt_offset(addr)]
        } else {
            self.chr[self.chr_offset(addr)]
        }
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x2000 {
            let off = self.nt_offset(addr);
            self.nt_ram[off] = val;
        } else if self.chr_is_ram {
            let off = self.chr_offset(addr);
            self.chr[off] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        Mirroring::FourScreen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::discrete::tests::banks;

    #[test]
    fn banks_and_nametable_pages() {
        let mut m = Gtrom::new(banks(32, 0x4000), vec![]);
        m.cpu_write(0x5000, 0x03);
        assert_eq!(m.cpu_read(0x8000), 6);
        m.cpu_write(0x8000, 0x0F); // not a register
        assert_eq!(m.cpu_read(0xC000), 7);
        // Four distinct nametables in page 0.
        for (i, nt) in [0x2000u16, 0x2400, 0x2800, 0x2C00].into_iter().enumerate() {
            assert_eq!(m.nt_target(nt), NtTarget::Cart);
            m.ppu_write(nt, i as u8 + 1);
        }
        assert_eq!(m.ppu_read(0x2C00), 4);
        m.cpu_write(0x7000, 0x20); // NT page 1
        assert_eq!(m.ppu_read(0x2000), 0);
        m.ppu_write(0x0005, 0x99);
        m.cpu_write(0x5000, 0x10); // CHR bank 1, NT page 0
        assert_eq!(m.ppu_read(0x0005), 0);
        assert_eq!(m.ppu_read(0x2400), 2);
    }
}
