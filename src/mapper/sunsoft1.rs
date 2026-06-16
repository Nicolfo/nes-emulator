use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Sunsoft-1 (mapper 184): fixed PRG with two switchable 4KB CHR banks. A
/// single write-only register sits in the $6000-$7FFF range:
///
/// ```text
/// 7  bit  0
/// .HHH .LLL
///  |    |
///  |    +--- bits 0-2: 4KB CHR bank at $0000
///  +-------- bits 4-6: 4KB CHR bank at $1000
/// ```
/// PRG ROM (16KB or 32KB) is fixed; mirroring is hardwired by the header.
/// Games: Atlantis no Nazo, Kanshakudama Nage Kantarou, Wing of Madoola.
#[derive(Serialize, Deserialize)]
pub struct Sunsoft1 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    chr_banks: [u8; 2],
    mirroring: Mirroring,
}

impl Sunsoft1 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Sunsoft1 {
            prg,
            chr,
            chr_is_ram,
            chr_banks: [0, 1],
            mirroring,
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x1000;
        let bank = self.chr_banks[(addr >> 12) as usize & 1] as usize % banks;
        bank * 0x1000 + (addr as usize & 0x0FFF)
    }
}

impl Mapper for Sunsoft1 {
    crate::impl_mapper_savestate!();

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[(addr as usize & 0x7FFF) % self.prg.len()]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        // The bank register is decoded in the $6000-$7FFF window.
        if (0x6000..=0x7FFF).contains(&addr) {
            self.chr_banks[0] = val & 0x07;
            self.chr_banks[1] = (val >> 4) & 0x07;
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

    fn sunsoft1() -> Sunsoft1 {
        // 32KB PRG, 8 CHR 4KB banks; byte = bank index.
        let prg: Vec<u8> = (0..0x8000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x1000).map(|i| (i / 0x1000) as u8).collect();
        Sunsoft1::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_fixed_32k() {
        let mut m = sunsoft1();
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xC000), 1);
    }

    #[test]
    fn chr_two_4k_banks() {
        let mut m = sunsoft1();
        m.cpu_write(0x6000, 0x32); // low bank 2, high bank 3
        assert_eq!(m.ppu_read(0x0000), 2);
        assert_eq!(m.ppu_read(0x1000), 3);
    }

    #[test]
    fn default_banks_sequential() {
        let mut m = sunsoft1();
        assert_eq!(m.ppu_read(0x0000), 0);
        assert_eq!(m.ppu_read(0x1000), 1);
    }

    #[test]
    fn prg_16k_mirrors() {
        let prg: Vec<u8> = (0..0x4000).map(|_| 0xAB).collect();
        let chr: Vec<u8> = vec![0; 0x2000];
        let mut m = Sunsoft1::new(prg, chr, Mirroring::Vertical);
        assert_eq!(m.cpu_read(0x8000), 0xAB);
        assert_eq!(m.cpu_read(0xC000), 0xAB); // 16KB mirrored into $C000
    }
}
