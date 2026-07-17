use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Irem G101 (mapper 32): two switchable 8KB PRG banks, eight 1KB CHR banks,
/// register-controlled mirroring and a PRG swap mode.
///
/// ```text
/// $8000-$8FFF  [...P PPPP]  PRG Reg 0 (8KB, at $8000 or $C000)
/// $9000-$9FFF  [.... ..MP]  M = mirroring (0=Vert, 1=Horz), P = PRG mode
/// $A000-$AFFF  [...P PPPP]  PRG Reg 1 (8KB @ $A000)
/// $B000-$B007  [CCCC CCCC]  CHR Regs 0-7 (1KB each), selected by addr & 7
/// ```
/// PRG mode 0: reg 0 @ $8000, second-last fixed @ $C000. Mode 1: second-last
/// fixed @ $8000, reg 0 @ $C000. The last 8KB is always fixed at $E000.
///
/// Submapper 1 (Major League) hardwires single-screen mirroring and ignores
/// the $9000 mirroring bit.
#[derive(Serialize, Deserialize)]
pub struct IremG101 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    /// Submapper 1: fixed single-screen, mirroring register ignored.
    fixed_mirroring: bool,
    prg_regs: [u8; 2],
    prg_mode: bool,
    chr_regs: [u8; 8],
}

impl IremG101 {
    pub fn new(submapper: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        let fixed_mirroring = submapper == 1 || mirroring == Mirroring::FourScreen;
        let mirroring = if submapper == 1 {
            Mirroring::SingleScreenLo
        } else {
            mirroring
        };
        IremG101 {
            prg,
            chr,
            chr_is_ram,
            mirroring,
            fixed_mirroring,
            prg_regs: [0; 2],
            prg_mode: false,
            chr_regs: [0; 8],
        }
    }

    fn prg_offset(&self, addr: u16) -> usize {
        let banks = self.prg.len() / 0x2000;
        let last = banks - 1;
        let bank = match addr {
            0x8000..=0x9FFF => {
                if self.prg_mode {
                    last - 1
                } else {
                    self.prg_regs[0] as usize % banks
                }
            }
            0xA000..=0xBFFF => self.prg_regs[1] as usize % banks,
            0xC000..=0xDFFF => {
                if self.prg_mode {
                    self.prg_regs[0] as usize % banks
                } else {
                    last - 1
                }
            }
            _ => last,
        };
        bank * 0x2000 + (addr as usize & 0x1FFF)
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x400;
        let bank = self.chr_regs[(addr >> 10) as usize & 7] as usize % banks;
        bank * 0x400 + (addr as usize & 0x3FF)
    }
}

impl Mapper for IremG101 {
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
        match addr {
            0x8000..=0x8FFF => self.prg_regs[0] = val,
            0x9000..=0x9FFF => {
                self.prg_mode = val & 0x02 != 0;
                if !self.fixed_mirroring {
                    self.mirroring = if val & 0x01 != 0 {
                        Mirroring::Horizontal
                    } else {
                        Mirroring::Vertical
                    };
                }
            }
            0xA000..=0xAFFF => self.prg_regs[1] = val,
            0xB000..=0xBFFF => self.chr_regs[(addr & 7) as usize] = val,
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

    fn g101() -> IremG101 {
        // 8 PRG 8KB banks (64KB), 16 CHR 1KB banks; byte = bank index.
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        IremG101::new(0, prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_mode_0_swaps_8000() {
        let mut m = g101();
        m.cpu_write(0x8000, 3); // reg 0
        m.cpu_write(0xA000, 4); // reg 1
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xA000), 4);
        assert_eq!(m.cpu_read(0xC000), 6); // second-last fixed
        assert_eq!(m.cpu_read(0xE000), 7); // last fixed
    }

    #[test]
    fn prg_mode_1_swaps_c000() {
        let mut m = g101();
        m.cpu_write(0x8000, 3); // reg 0
        m.cpu_write(0x9000, 0x02); // PRG mode 1
        assert_eq!(m.cpu_read(0x8000), 6); // second-last fixed
        assert_eq!(m.cpu_read(0xC000), 3); // reg 0 moves here
        assert_eq!(m.cpu_read(0xE000), 7); // last fixed
    }

    #[test]
    fn chr_banking() {
        let mut m = g101();
        m.cpu_write(0xB000, 9); // 1KB @ $0000
        m.cpu_write(0xB007, 2); // 1KB @ $1C00
        assert_eq!(m.ppu_read(0x0000), 9);
        assert_eq!(m.ppu_read(0x1C00), 2);
    }

    #[test]
    fn mirroring_toggle() {
        let mut m = g101();
        m.cpu_write(0x9000, 0x00);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0x9000, 0x01);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn submapper_1_fixes_single_screen() {
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        let mut m = IremG101::new(1, prg, chr, Mirroring::Vertical);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        m.cpu_write(0x9000, 0x01); // would set Horizontal, but ignored
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
    }

    #[test]
    fn chr_ram_fallback_rw() {
        let prg: Vec<u8> = (0..2 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let mut m = IremG101::new(0, prg, vec![], Mirroring::Vertical);
        m.ppu_write(0x1234, 0xAB);
        assert_eq!(m.ppu_read(0x1234), 0xAB);
    }
}
