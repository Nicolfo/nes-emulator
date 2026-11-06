use super::discrete::{bank_index, chr_or_ram};
use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Mapper 246 (G0151-1, Fong Shen Bang - Zhu Lu Zhi Zhan). Registers at
/// $6000-$6003 select 8KB PRG banks for $8000/$A000/$C000/$E000 and
/// $6004-$6007 2KB CHR banks for $0000/$0800/$1000/$1800. 2KB of
/// battery-backed RAM sits at $6800-$6FFF. Reads in the vector area
/// (addresses with all of $FFE4's bits set: $FFE4-7, $FFEC-F, $FFF4-7,
/// $FFFC-F) always come from the last PRG bank, and $E000 powers on
/// pointing at it.
#[derive(Serialize, Deserialize)]
pub struct FongShenBang {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    regs: [u8; 8],
    mirroring: Mirroring,
}

impl FongShenBang {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let (chr, chr_is_ram) = chr_or_ram(chr, 0x2000);
        let mut regs = [0u8; 8];
        regs[3] = 0xFF;
        FongShenBang {
            prg,
            chr,
            chr_is_ram,
            prg_ram: vec![0; 0x800],
            regs,
            mirroring,
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let slot = (addr as usize >> 11) & 3;
        bank_index(
            self.chr.len(),
            0x800,
            self.regs[4 + slot] as usize,
            addr as usize,
        )
    }
}

impl Mapper for FongShenBang {
    crate::impl_mapper_savestate!(chr, prg_ram);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        // The board's 2KB work RAM is fixed; NES 2.0 rounds it to 8KB.
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let len = self.prg.len();
        let bank = if addr & 0xFFE4 == 0xFFE4 {
            len / 0x2000 - 1
        } else {
            self.regs[(addr as usize >> 13) & 3] as usize
        };
        self.prg[bank_index(len, 0x2000, bank, addr as usize)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x6000..=0x67FF => self.regs[(addr & 7) as usize] = val,
            0x6800..=0x6FFF => self.prg_ram[(addr & 0x7FF) as usize] = val,
            _ => {}
        }
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        (0x6800..=0x6FFF)
            .contains(&addr)
            .then(|| self.prg_ram[(addr & 0x7FF) as usize])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.prg_ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.prg_ram)
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
    use crate::mapper::discrete::tests::banks;

    #[test]
    fn prg_chr_banks_ram_and_vectors() {
        let mut m = FongShenBang::new(banks(16, 0x2000), banks(16, 0x800), Mirroring::Vertical);
        assert_eq!(m.cpu_read(0xE000), 15, "$E000 powers on at the last bank");
        m.cpu_write(0x6000, 2);
        m.cpu_write(0x6003, 4);
        m.cpu_write(0x6006, 9);
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xE000), 4);
        assert_eq!(m.cpu_read(0xFFFC), 15, "vectors from the last bank");
        assert_eq!(m.ppu_read(0x1000), 9);
        m.cpu_write(0x6810, 0x5A);
        assert_eq!(m.prg_ram_read(0x6810), Some(0x5A));
        assert_eq!(m.prg_ram_read(0x6010), None);
    }
}
