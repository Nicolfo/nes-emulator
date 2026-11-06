use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Jaleco boards with a single bank latch at $6000-$7FFF (no PRG RAM):
///
/// - **86 (JF-13)**: `$6000-$6FFF [.CPP ..CC]`: 32KB PRG bank `PP`, 8KB CHR
///   bank `C..CC` (bit 6 is CHR bit 2). $7000-$7FFF drives the uPD7756
///   speech chip (not emulated). Moero!! Pro Yakyuu, Argus (J).
/// - **87 (J87, also Konami/Taito)**: `$6000-$7FFF [.... ..LH]`: 8KB CHR bank
///   with the two bits swapped (`HL`). PRG is fixed (NROM). City Connection
///   (J), The Goonies (J), Argus, Hyper Olympic.
/// - **140 (JF-11/JF-14)**: `$6000-$7FFF [..PP CCCC]`: 32KB PRG bank and
///   8KB CHR bank. Bio Senshi Dan, Mississippi Satsujin Jiken.
#[derive(Serialize, Deserialize)]
pub struct JalecoLatch {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mapper_id: u8,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl JalecoLatch {
    pub fn new(mapper_id: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        JalecoLatch {
            prg,
            chr,
            chr_is_ram,
            mapper_id,
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

impl Mapper for JalecoLatch {
    crate::impl_mapper_savestate!(chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        // 32KB banking; a 16KB image mirrors into both halves.
        let off = self.prg_bank as usize * 0x8000 + (addr as usize & 0x7FFF);
        self.prg[off % self.prg.len()]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match (self.mapper_id, addr) {
            (86, 0x6000..=0x6FFF) => {
                self.prg_bank = (val >> 4) & 3;
                self.chr_bank = (val & 3) | ((val >> 4) & 4);
            }
            (87, 0x6000..=0x7FFF) => {
                self.chr_bank = ((val & 1) << 1) | ((val >> 1) & 1);
            }
            (140, 0x6000..=0x7FFF) => {
                self.prg_bank = (val >> 4) & 3;
                self.chr_bank = val & 0x0F;
            }
            _ => {}
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

    fn board(id: u8) -> JalecoLatch {
        let prg: Vec<u8> = (0..4 * 0x8000).map(|i| (i / 0x8000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        JalecoLatch::new(id, prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn jf13_prg_and_split_chr_bits() {
        let mut m = board(86);
        m.cpu_write(0x6000, 0x40 | 0x20 | 0x01); // CHR bit2, PRG 2, CHR 1
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.ppu_read(0x0000), 5);
        m.cpu_write(0x7000, 0xFF); // speech port: no banking effect
        assert_eq!(m.cpu_read(0x8000), 2);
    }

    #[test]
    fn j87_swaps_chr_bits() {
        let mut m = board(87);
        m.cpu_write(0x6000, 0x01);
        assert_eq!(m.ppu_read(0x0000), 2);
        m.cpu_write(0x7FFF, 0x02);
        assert_eq!(m.ppu_read(0x0000), 1);
        assert_eq!(m.cpu_read(0x8000), 0, "PRG fixed");
    }

    #[test]
    fn jf11_prg_and_chr() {
        let mut m = board(140);
        m.cpu_write(0x6000, 0x37);
        assert_eq!(m.cpu_read(0xFFFF), 3);
        assert_eq!(m.ppu_read(0x1FFF), 7);
    }

    #[test]
    fn sixteen_kb_prg_mirrors() {
        let prg: Vec<u8> = (0..0x4000).map(|i| (i >> 8) as u8).collect();
        let mut m = JalecoLatch::new(87, prg, vec![], Mirroring::Horizontal);
        assert_eq!(m.cpu_read(0x8100), m.cpu_read(0xC100));
        m.ppu_write(0x0010, 0x99); // CHR RAM
        assert_eq!(m.ppu_read(0x0010), 0x99);
    }
}
