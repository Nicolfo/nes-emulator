use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// MMC2 (mapper 9, Punch-Out!!): 8KB PRG banking with the top three banks
/// fixed, and dual 4KB CHR banks selected by latches that flip when the PPU
/// fetches the magic tiles $FD/$FE.
#[derive(Serialize, Deserialize)]
pub struct Mmc2 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    #[serde(default)]
    chr_is_ram: bool,
    mirroring: Mirroring,
    prg_bank: u8,
    // CHR bank registers: [FD $0000, FE $0000, FD $1000, FE $1000].
    chr_regs: [u8; 4],
    // Latch per pattern table; true selects the FE register.
    latch: [bool; 2],
}

impl Mmc2 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Mmc2 {
            prg,
            chr,
            chr_is_ram,
            mirroring,
            prg_bank: 0,
            chr_regs: [0; 4],
            latch: [true; 2],
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let table = (addr >> 12) as usize & 1;
        let reg = table * 2 + self.latch[table] as usize;
        let banks = self.chr.len() / 0x1000;
        (self.chr_regs[reg] as usize % banks) * 0x1000 + (addr as usize & 0xFFF)
    }

    /// Latch 0 flips on the exact addresses $0FD8/$0FE8; latch 1 on the
    /// ranges $1FD8-$1FDF/$1FE8-$1FEF. The flip lands after the fetch.
    fn update_latch(&mut self, addr: u16) {
        match addr {
            0x0FD8 => self.latch[0] = false,
            0x0FE8 => self.latch[0] = true,
            0x1FD8..=0x1FDF => self.latch[1] = false,
            0x1FE8..=0x1FEF => self.latch[1] = true,
            _ => {}
        }
    }
}

impl Mapper for Mmc2 {
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
        // The fixed-top-three layout needs at least 32KB; mirror smaller
        // (degenerate) images instead of underflowing the bank math.
        if self.prg.len() < 0x8000 {
            return self.prg[addr as usize & (self.prg.len() - 1)];
        }
        let banks = self.prg.len() / 0x2000;
        let bank = if addr < 0xA000 {
            self.prg_bank as usize % banks
        } else {
            // Last three 8KB banks fixed at $A000-$FFFF.
            banks - 3 + ((addr as usize - 0xA000) >> 13)
        };
        self.prg[bank * 0x2000 + (addr as usize & 0x1FFF)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0xA000..=0xAFFF => self.prg_bank = val & 0x0F,
            0xB000..=0xBFFF => self.chr_regs[0] = val & 0x1F,
            0xC000..=0xCFFF => self.chr_regs[1] = val & 0x1F,
            0xD000..=0xDFFF => self.chr_regs[2] = val & 0x1F,
            0xE000..=0xEFFF => self.chr_regs[3] = val & 0x1F,
            // A four-screen board ignores the mirroring register.
            0xF000..=0xFFFF if self.mirroring != Mirroring::FourScreen => {
                self.mirroring = if val & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let v = self.chr[self.chr_offset(addr)];
        self.update_latch(addr);
        v
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        // Real MMC2 boards are CHR ROM; the RAM path only serves zero-CHR
        // images (iNES CHR RAM convention). Writes don't clock the latch.
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

    fn mmc2() -> Mmc2 {
        // 8 PRG banks (64KB), 8 CHR 4KB banks (32KB); byte = bank index.
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x1000).map(|i| (i / 0x1000) as u8).collect();
        Mmc2::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_banking() {
        let mut m = mmc2();
        m.cpu_write(0xA000, 3);
        assert_eq!(m.cpu_read(0x8000), 3);
        // Last three banks fixed.
        assert_eq!(m.cpu_read(0xA000), 5);
        assert_eq!(m.cpu_read(0xC000), 6);
        assert_eq!(m.cpu_read(0xE000), 7);
    }

    #[test]
    fn latch_flips_after_fetch() {
        let mut m = mmc2();
        m.cpu_write(0xB000, 1); // FD $0000
        m.cpu_write(0xC000, 2); // FE $0000
        // Latch starts at FE.
        assert_eq!(m.ppu_read(0x0000), 2);
        // $0FD8 fetch serves the FE bank, then flips to FD.
        assert_eq!(m.ppu_read(0x0FD8), 2);
        assert_eq!(m.ppu_read(0x0000), 1);
        // $0FE8 flips back to FE.
        m.ppu_read(0x0FE8);
        assert_eq!(m.ppu_read(0x0000), 2);
    }

    #[test]
    fn latch1_uses_ranges() {
        let mut m = mmc2();
        m.cpu_write(0xD000, 3); // FD $1000
        m.cpu_write(0xE000, 4); // FE $1000
        assert_eq!(m.ppu_read(0x1000), 4);
        m.ppu_read(0x1FDC); // anywhere in $1FD8-$1FDF
        assert_eq!(m.ppu_read(0x1000), 3);
        m.ppu_read(0x1FEF);
        assert_eq!(m.ppu_read(0x1000), 4);
        // Latch 0 only flips on the exact addresses; $0FD9 must not.
        m.cpu_write(0xB000, 1);
        m.cpu_write(0xC000, 2);
        m.ppu_read(0x0FD9);
        assert_eq!(m.ppu_read(0x0000), 2);
    }

    #[test]
    fn mirroring_register() {
        let mut m = mmc2();
        m.cpu_write(0xF000, 1);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0xF000, 0);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
    }
}
