use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Color Dreams (mapper 11): 32KB PRG and 8KB CHR selected by one register.
/// The board has no bus-conflict prevention, so the written value is ANDed
/// with the ROM byte at the written address. Headers declaring no CHR ROM
/// get 8KB of unbanked CHR RAM.
#[derive(Serialize, Deserialize)]
pub struct ColorDreams {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    prg_bank: u8,
    chr_bank: u8,
}

impl ColorDreams {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        ColorDreams {
            prg,
            chr,
            chr_is_ram,
            mirroring,
            prg_bank: 0,
            chr_bank: 0,
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x2000;
        (self.chr_bank as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)
    }

    fn prg_offset(&self, addr: u16) -> usize {
        // A 16KB image can't fill this board's 32KB window; mirror it (the
        // loader guarantees at least one whole 16KB bank).
        if self.prg.len() < 0x8000 {
            return addr as usize & (self.prg.len() - 1);
        }
        let banks = self.prg.len() / 0x8000;
        (self.prg_bank as usize % banks) * 0x8000 + (addr as usize & 0x7FFF)
    }
}

impl Mapper for ColorDreams {
    crate::impl_mapper_savestate!(prg, chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            let val = val & self.prg[self.prg_offset(addr)];
            self.prg_bank = val & 0x03;
            self.chr_bank = val >> 4;
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

    fn mapper_with_rom_byte(b: u8) -> ColorDreams {
        // 4 x 32KB PRG filled with `b`, 16 x 8KB CHR; CHR byte = bank index.
        let prg = vec![b; 4 * 0x8000];
        let chr: Vec<u8> = (0..16 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        ColorDreams::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn selects_prg_and_chr() {
        let mut m = mapper_with_rom_byte(0xFF);
        m.cpu_write(0x8000, 0x52); // CHR 5, PRG 2
        assert_eq!(m.ppu_read(0x0000), 5);
        // PRG bank 2 of an all-0xFF ROM still reads 0xFF; check via offset.
        assert_eq!(m.prg_offset(0x8000), 2 * 0x8000);
    }

    #[test]
    fn chr_ram_when_no_chr_rom() {
        let mut m = ColorDreams::new(vec![0xFF; 0x8000], vec![], Mirroring::Vertical);
        m.ppu_write(0x0123, 0x42);
        assert_eq!(m.ppu_read(0x0123), 0x42);
    }

    #[test]
    fn bus_conflict_ands_with_rom() {
        let mut m = mapper_with_rom_byte(0x21);
        m.cpu_write(0x8000, 0xFF); // 0xFF & 0x21 = 0x21: CHR 2, PRG 1
        assert_eq!(m.ppu_read(0x0000), 2);
        assert_eq!(m.prg_offset(0x8000), 0x8000);
    }
}
