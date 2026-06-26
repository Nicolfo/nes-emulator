use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Sunsoft-3 (mapper 67): one switchable 16KB PRG bank with the last fixed,
/// four 2KB CHR banks, register-controlled mirroring and a 16-bit CPU-cycle
/// IRQ counter.
///
/// ```text
/// $8800  [CCCC CCCC]  CHR Reg 0 (2KB @ $0000)
/// $9800  [CCCC CCCC]  CHR Reg 1 (2KB @ $0800)
/// $A800  [CCCC CCCC]  CHR Reg 2 (2KB @ $1000)
/// $B800  [CCCC CCCC]  CHR Reg 3 (2KB @ $1800)
/// $C800  [IIII IIII]  IRQ counter (16-bit; first write high byte, then low)
/// $D800  [...E ....]  E = IRQ enable; the write also acks and resets the toggle
/// $E800  [.... ..MM]  mirroring (0=Vert, 1=Horz, 2=1ScrA, 3=1ScrB)
/// $F800  [.... PPPP]  PRG Reg (16KB @ $8000)
/// ```
/// Games: Fantasy Zone 2, Mito Koumon, Nantettatte!! Baseball.
#[derive(Clone, Serialize, Deserialize)]
pub struct Sunsoft3 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    four_screen: bool,
    prg_bank: u8,
    chr_banks: [u8; 4],
    irq_enabled: bool,
    irq_counter: u16,
    /// Which byte the next $C800 write targets: false = high, true = low.
    irq_toggle: bool,
    irq_line: bool,
}

impl Sunsoft3 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        let four_screen = mirroring == Mirroring::FourScreen;
        Sunsoft3 {
            prg,
            chr,
            chr_is_ram,
            mirroring,
            four_screen,
            prg_bank: 0,
            chr_banks: [0; 4],
            irq_enabled: false,
            irq_counter: 0,
            irq_toggle: false,
            irq_line: false,
        }
    }

    fn prg_offset(&self, addr: u16) -> usize {
        let banks = self.prg.len() / 0x4000;
        let bank = if addr < 0xC000 {
            self.prg_bank as usize % banks
        } else {
            banks - 1
        };
        bank * 0x4000 + (addr as usize & 0x3FFF)
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x800;
        let bank = self.chr_banks[(addr >> 11) as usize & 3] as usize % banks;
        bank * 0x800 + (addr as usize & 0x7FF)
    }
}

impl Mapper for Sunsoft3 {
    crate::impl_mapper_savestate!(chr_is_ram = chr_is_ram);

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr & 0xF800 {
            0x8800 => self.chr_banks[0] = val,
            0x9800 => self.chr_banks[1] = val,
            0xA800 => self.chr_banks[2] = val,
            0xB800 => self.chr_banks[3] = val,
            0xC800 => {
                if self.irq_toggle {
                    self.irq_counter = (self.irq_counter & 0xFF00) | val as u16;
                } else {
                    self.irq_counter = (self.irq_counter & 0x00FF) | ((val as u16) << 8);
                }
                self.irq_toggle = !self.irq_toggle;
            }
            0xD800 => {
                self.irq_enabled = val & 0x10 != 0;
                self.irq_toggle = false;
                self.irq_line = false;
            }
            0xE800 => {
                if !self.four_screen {
                    self.mirroring = match val & 3 {
                        0 => Mirroring::Vertical,
                        1 => Mirroring::Horizontal,
                        2 => Mirroring::SingleScreenLo,
                        _ => Mirroring::SingleScreenHi,
                    };
                }
            }
            0xF800 => self.prg_bank = val,
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

    fn irq(&self) -> bool {
        self.irq_line
    }

    fn cpu_clock(&mut self) {
        if self.irq_enabled {
            let (next, wrapped) = self.irq_counter.overflowing_sub(1);
            self.irq_counter = next;
            if wrapped {
                self.irq_line = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sunsoft3() -> Sunsoft3 {
        // 4 PRG 16KB banks, 8 CHR 2KB banks; byte = bank index.
        let prg: Vec<u8> = (0..4 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x800).map(|i| (i / 0x800) as u8).collect();
        Sunsoft3::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_switch_and_fixed() {
        let mut m = sunsoft3();
        m.cpu_write(0xF800, 2);
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xC000), 3); // last fixed
    }

    #[test]
    fn chr_banking() {
        let mut m = sunsoft3();
        m.cpu_write(0x8800, 1);
        m.cpu_write(0xB800, 7);
        assert_eq!(m.ppu_read(0x0000), 1);
        assert_eq!(m.ppu_read(0x1800), 7);
    }

    #[test]
    fn mirroring_modes() {
        let mut m = sunsoft3();
        m.cpu_write(0xE800, 1);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0xE800, 2);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        m.cpu_write(0xE800, 3);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
    }

    #[test]
    fn irq_16bit_cycle_counter() {
        let mut m = sunsoft3();
        m.cpu_write(0xC800, 0x00); // high byte
        m.cpu_write(0xC800, 0x05); // low byte -> counter = 5
        m.cpu_write(0xD800, 0x10); // enable
        for i in 0..6 {
            assert!(!m.irq(), "IRQ too early at cycle {i}");
            m.cpu_clock();
        }
        assert!(m.irq()); // wrapped 0 -> $FFFF
        m.cpu_write(0xD800, 0x00); // ack + disable
        assert!(!m.irq());
    }

    #[test]
    fn irq_disabled_does_not_count() {
        let mut m = sunsoft3();
        m.cpu_write(0xC800, 0x00);
        m.cpu_write(0xC800, 0x01);
        for _ in 0..10 {
            m.cpu_clock();
        }
        assert!(!m.irq());
    }
}
