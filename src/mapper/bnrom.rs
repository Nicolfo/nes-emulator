use super::{Mapper, Mirroring};

/// Mapper 34 covers two unrelated boards distinguished by whether the header
/// declares CHR ROM:
///
/// * **BNROM** (no CHR ROM): 32KB PRG bank selected by any write to
///   $8000-$FFFF, with 8KB unbanked CHR RAM. Deadly Towers, Mashou.
/// * **NINA-001** (CHR ROM present): 8KB PRG RAM at $6000-$7FFF, a 32KB PRG
///   bank at $7FFD and two 4KB CHR banks at $7FFE/$7FFF. Impossible Mission II.
///   The register writes pass through to PRG RAM as well, since they share the
///   $6000-$7FFF window.
pub struct Bnrom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_ram: [u8; 0x2000],
    mirroring: Mirroring,
    nina: bool,
    prg_bank: u8,
    chr_low: u8,
    chr_high: u8,
}

impl Bnrom {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Bnrom {
            prg,
            chr,
            chr_is_ram,
            prg_ram: [0; 0x2000],
            mirroring,
            // CHR ROM present => NINA-001; otherwise BNROM with CHR RAM.
            nina: !chr_is_ram,
            prg_bank: 0,
            chr_low: 0,
            chr_high: 1,
        }
    }

    fn prg_offset(&self, addr: u16) -> usize {
        let banks = self.prg.len() / 0x8000;
        (self.prg_bank as usize % banks) * 0x8000 + (addr as usize & 0x7FFF)
    }

    fn chr_offset(&self, addr: u16) -> usize {
        if self.chr_is_ram {
            return addr as usize & 0x1FFF;
        }
        let banks = self.chr.len() / 0x1000;
        let bank = if addr & 0x1000 != 0 {
            self.chr_high
        } else {
            self.chr_low
        };
        (bank as usize % banks) * 0x1000 + (addr as usize & 0xFFF)
    }
}

impl Mapper for Bnrom {
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if self.nina {
            // Registers live inside the PRG RAM window and write through.
            if (0x6000..=0x7FFF).contains(&addr) {
                self.prg_ram[(addr & 0x1FFF) as usize] = val;
                match addr {
                    0x7FFD => self.prg_bank = val,
                    0x7FFE => self.chr_low = val,
                    0x7FFF => self.chr_high = val,
                    _ => {}
                }
            }
        } else if addr >= 0x8000 {
            // BNROM: any high write selects the 32KB PRG bank.
            self.prg_bank = val;
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

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        if self.nina {
            Some(self.prg_ram[(addr & 0x1FFF) as usize])
        } else {
            None
        }
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        self.nina.then_some(&self.prg_ram[..])
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        if self.nina {
            Some(&mut self.prg_ram)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bnrom_32k_bank_switch() {
        // 4 x 32KB PRG, no CHR ROM (CHR RAM); PRG byte = 32KB bank index.
        let prg: Vec<u8> = (0..4 * 0x8000).map(|i| (i / 0x8000) as u8).collect();
        let mut m = Bnrom::new(prg, vec![], Mirroring::Vertical);
        assert_eq!(m.cpu_read(0x8000), 0);
        m.cpu_write(0x8000, 2);
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xFFFF), 2);
        // CHR RAM is read/write and unbanked.
        m.ppu_write(0x1234, 0xCD);
        assert_eq!(m.ppu_read(0x1234), 0xCD);
        // No PRG RAM on BNROM.
        assert_eq!(m.prg_ram_read(0x6000), None);
    }

    #[test]
    fn nina_prg_and_chr_banks() {
        // 2 x 32KB PRG; 8 x 4KB CHR; CHR byte = 4KB bank index.
        let prg: Vec<u8> = (0..2 * 0x8000).map(|i| (i / 0x8000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x1000).map(|i| (i / 0x1000) as u8).collect();
        let mut m = Bnrom::new(prg, chr, Mirroring::Horizontal);
        m.cpu_write(0x7FFD, 1); // PRG 32KB bank 1
        m.cpu_write(0x7FFE, 3); // CHR $0000 = 4KB bank 3
        m.cpu_write(0x7FFF, 5); // CHR $1000 = 4KB bank 5
        assert_eq!(m.cpu_read(0x8000), 1);
        assert_eq!(m.ppu_read(0x0000), 3);
        assert_eq!(m.ppu_read(0x1000), 5);
        // Register writes pass through to PRG RAM.
        assert_eq!(m.prg_ram_read(0x7FFD), Some(1));
    }
}
