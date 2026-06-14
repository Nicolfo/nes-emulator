use super::{Mapper, Mirroring};

/// MMC4 (mapper 10, Fire Emblem): the MMC2 CHR-latch board scaled up to 16KB
/// PRG banking with the last 16KB fixed, plus 8KB of (usually battery-backed)
/// PRG RAM. The dual 4KB CHR banks are selected by latches that flip when the
/// PPU fetches the magic tiles $FD/$FE — identical to MMC2.
pub struct Mmc4 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: [u8; 0x2000],
    mirroring: Mirroring,
    prg_bank: u8,
    // CHR bank registers: [FD $0000, FE $0000, FD $1000, FE $1000].
    chr_regs: [u8; 4],
    // Latch per pattern table; true selects the FE register.
    latch: [bool; 2],
}

impl Mmc4 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        Mmc4 {
            prg,
            chr,
            prg_ram: [0; 0x2000],
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

impl Mapper for Mmc4 {
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let banks = self.prg.len() / 0x4000;
        let bank = if addr < 0xC000 {
            self.prg_bank as usize % banks
        } else {
            // Last 16KB bank fixed at $C000-$FFFF.
            banks - 1
        };
        self.prg[bank * 0x4000 + (addr as usize & 0x3FFF)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x6000..=0x7FFF => self.prg_ram[(addr & 0x1FFF) as usize] = val,
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

    fn ppu_write(&mut self, _addr: u16, _val: u8) {
        // CHR is always ROM on MMC4 boards.
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        Some(self.prg_ram[(addr & 0x1FFF) as usize])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.prg_ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.prg_ram)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mmc4() -> Mmc4 {
        // 4 PRG banks (64KB, 16KB each), 8 CHR 4KB banks (32KB); byte = bank index.
        let prg: Vec<u8> = (0..4 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x1000).map(|i| (i / 0x1000) as u8).collect();
        Mmc4::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_banking() {
        let mut m = mmc4();
        m.cpu_write(0xA000, 2);
        assert_eq!(m.cpu_read(0x8000), 2);
        // Last 16KB bank fixed.
        assert_eq!(m.cpu_read(0xC000), 3);
        assert_eq!(m.cpu_read(0xE000), 3);
    }

    #[test]
    fn latch_flips_after_fetch() {
        let mut m = mmc4();
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
        let mut m = mmc4();
        m.cpu_write(0xD000, 3); // FD $1000
        m.cpu_write(0xE000, 4); // FE $1000
        assert_eq!(m.ppu_read(0x1000), 4);
        m.ppu_read(0x1FDC); // anywhere in $1FD8-$1FDF
        assert_eq!(m.ppu_read(0x1000), 3);
        m.ppu_read(0x1FEF);
        assert_eq!(m.ppu_read(0x1000), 4);
    }

    #[test]
    fn mirroring_register() {
        let mut m = mmc4();
        m.cpu_write(0xF000, 1);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0xF000, 0);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn prg_ram_read_write() {
        let mut m = mmc4();
        m.cpu_write(0x6123, 0xAB);
        assert_eq!(m.prg_ram_read(0x6123), Some(0xAB));
    }
}
