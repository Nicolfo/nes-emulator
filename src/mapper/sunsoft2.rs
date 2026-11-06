use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Sunsoft-2 boards: a 16KB PRG bank at $8000 (last bank fixed at $C000)
/// selected by a latch at $8000-$FFFF.
///
/// - **89 (Sunsoft-2 on the Sunsoft-3 board)**: `[CPPP MCCC]` - PRG bank
///   `PPP`, 8KB CHR bank `C...CCC` (bit 7 is CHR bit 3), `M` one-screen
///   nametable select. Tenka no Goikenban: Mito Koumon.
/// - **93 (Sunsoft-2 on the Sunsoft-3R board)**: `[.PPP ...E]` - PRG bank,
///   `E` enables the 8KB CHR RAM (games always set it); mirroring is
///   hardwired. Fantasy Zone (J), Shanghai.
#[derive(Serialize, Deserialize)]
pub struct Sunsoft2 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mapper_id: u8,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Sunsoft2 {
    pub fn new(mapper_id: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Sunsoft2 {
            prg,
            chr,
            chr_is_ram,
            mapper_id,
            prg_bank: 0,
            chr_bank: 0,
            mirroring: if mapper_id == 89 {
                Mirroring::SingleScreenLo
            } else {
                mirroring
            },
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x2000;
        (self.chr_bank as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)
    }
}

impl Mapper for Sunsoft2 {
    crate::impl_mapper_savestate!(chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        let banks = self.prg.len() / 0x4000;
        let bank = match addr {
            0x8000..=0xBFFF => self.prg_bank as usize % banks,
            0xC000..=0xFFFF => banks - 1,
            _ => return 0,
        };
        self.prg[bank * 0x4000 + (addr as usize & 0x3FFF)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr < 0x8000 {
            return;
        }
        self.prg_bank = (val >> 4) & 7;
        if self.mapper_id == 89 {
            self.chr_bank = (val & 7) | ((val >> 4) & 8);
            self.mirroring = if val & 0x08 != 0 {
                Mirroring::SingleScreenHi
            } else {
                Mirroring::SingleScreenLo
            };
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

    fn prg() -> Vec<u8> {
        (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect()
    }

    #[test]
    fn mapper89_prg_chr_and_one_screen() {
        let chr: Vec<u8> = (0..16 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let mut m = Sunsoft2::new(89, prg(), chr, Mirroring::Horizontal);
        m.cpu_write(0x8000, 0x80 | 0x30 | 0x08 | 0x05);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xC000), 7);
        assert_eq!(m.ppu_read(0x0000), 13);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
        m.cpu_write(0x8000, 0x00);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
    }

    #[test]
    fn mapper93_prg_only_with_chr_ram() {
        let mut m = Sunsoft2::new(93, prg(), vec![], Mirroring::Vertical);
        m.cpu_write(0xC000, 0x51);
        assert_eq!(m.cpu_read(0x8000), 5);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.ppu_write(0x0123, 0x42);
        assert_eq!(m.ppu_read(0x0123), 0x42);
    }
}
