use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Jaleco SS88006 (mapper 18): three switchable 8KB PRG banks plus the last
/// fixed, eight 1KB CHR banks, 8KB PRG RAM with enable/write-protect,
/// register-controlled mirroring and a CPU-cycle IRQ counter whose width can
/// be cut to 4, 8 or 12 bits. Every bank register is written as two nibbles.
///
/// ```text
/// $8000/$8001  PRG bank 0 ($8000) low / high nibble
/// $8002/$8003  PRG bank 1 ($A000)
/// $9000/$9001  PRG bank 2 ($C000)
/// $9002        [.... ..WE] PRG RAM: E = enable, W = write enable
/// $A000-$D003  CHR banks 0-7 (1KB), low / high nibble pairs
/// $E000-$E003  IRQ reload value, nibbles 0-3
/// $F000        IRQ acknowledge + reload counter
/// $F001        [.... SSSE] E = IRQ enable, S = counter width (also acks)
/// $F002        [.... ..MM] mirroring: 0 H, 1 V, 2 1ScA, 3 1ScB
/// $F003        uPD7756 speech control (not emulated)
/// ```
/// Games: Pizza Pop!, Ninja Jajamaru: Ginga Daisakusen, Plasma Ball,
/// The Lord of King, Saiyuuki World 2.
#[derive(Serialize, Deserialize)]
pub struct JalecoSs88006 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    prg_ram_enabled: bool,
    prg_ram_writable: bool,
    prg_banks: [u8; 3],
    chr_banks: [u8; 8],
    mirroring: Mirroring,
    four_screen: bool,
    irq_reload: u16,
    irq_counter: u16,
    /// Active counter bits: $FFFF, $0FFF, $00FF or $000F.
    irq_mask: u16,
    irq_enabled: bool,
    irq_line: bool,
}

impl JalecoSs88006 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        JalecoSs88006 {
            prg,
            chr,
            chr_is_ram,
            prg_ram: vec![0; 0x2000],
            prg_ram_enabled: false,
            prg_ram_writable: false,
            prg_banks: [0, 1, 2],
            chr_banks: [0; 8],
            four_screen: mirroring == Mirroring::FourScreen,
            mirroring,
            irq_reload: 0,
            irq_counter: 0,
            irq_mask: 0xFFFF,
            irq_enabled: false,
            irq_line: false,
        }
    }

    fn prg_offset(&self, addr: u16) -> usize {
        let banks = self.prg.len() / 0x2000;
        let bank = match addr {
            0x8000..=0x9FFF => self.prg_banks[0] as usize,
            0xA000..=0xBFFF => self.prg_banks[1] as usize,
            0xC000..=0xDFFF => self.prg_banks[2] as usize,
            _ => banks - 1,
        } % banks;
        bank * 0x2000 + (addr as usize & 0x1FFF)
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x400;
        let bank = self.chr_banks[(addr as usize >> 10) & 7] as usize % banks;
        bank * 0x400 + (addr as usize & 0x3FF)
    }
}

/// Replace the low (`high == false`) or high nibble of `reg` with `val`.
fn set_nibble(reg: &mut u8, val: u8, high: bool) {
    *reg = if high {
        (*reg & 0x0F) | (val & 0x0F) << 4
    } else {
        (*reg & 0xF0) | (val & 0x0F)
    };
}

impl Mapper for JalecoSs88006 {
    crate::impl_mapper_savestate!(chr, prg_ram);

    fn set_ram_sizes(&mut self, prg_ram: usize, chr_ram: usize) {
        if prg_ram > 0 {
            self.prg_ram = vec![0; prg_ram];
        }
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
        if (0x6000..=0x7FFF).contains(&addr) {
            if self.prg_ram_enabled && self.prg_ram_writable {
                let len = self.prg_ram.len();
                self.prg_ram[(addr as usize - 0x6000) % len] = val;
            }
            return;
        }
        if addr < 0x8000 {
            return;
        }
        let high = addr & 1 != 0;
        match addr & 0xF003 {
            0x8000 | 0x8001 => set_nibble(&mut self.prg_banks[0], val, high),
            0x8002 | 0x8003 => set_nibble(&mut self.prg_banks[1], val, high),
            0x9000 | 0x9001 => set_nibble(&mut self.prg_banks[2], val, high),
            0x9002 => {
                self.prg_ram_enabled = val & 1 != 0;
                self.prg_ram_writable = val & 2 != 0;
            }
            a @ 0xA000..=0xD003 => {
                // Two registers per $1000 page: $x000/1 and $x002/3.
                let reg = (((a - 0xA000) >> 12) * 2 + ((a >> 1) & 1)) as usize;
                set_nibble(&mut self.chr_banks[reg], val, high);
            }
            a @ 0xE000..=0xE003 => {
                let shift = (a & 3) * 4;
                self.irq_reload =
                    (self.irq_reload & !(0xF << shift)) | ((val as u16 & 0xF) << shift);
            }
            0xF000 => {
                self.irq_line = false;
                self.irq_counter = self.irq_reload;
            }
            0xF001 => {
                self.irq_line = false;
                self.irq_enabled = val & 1 != 0;
                self.irq_mask = if val & 0x08 != 0 {
                    0x000F
                } else if val & 0x04 != 0 {
                    0x00FF
                } else if val & 0x02 != 0 {
                    0x0FFF
                } else {
                    0xFFFF
                };
            }
            0xF002 if !self.four_screen => {
                self.mirroring = match val & 3 {
                    0 => Mirroring::Horizontal,
                    1 => Mirroring::Vertical,
                    2 => Mirroring::SingleScreenLo,
                    _ => Mirroring::SingleScreenHi,
                };
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

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        self.prg_ram_enabled
            .then(|| self.prg_ram[(addr as usize - 0x6000) % self.prg_ram.len()])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.prg_ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.prg_ram)
    }

    fn irq(&self) -> bool {
        self.irq_line
    }

    fn cpu_clock(&mut self) {
        if !self.irq_enabled {
            return;
        }
        // Only the masked low bits count; the upper bits hold still.
        let m = self.irq_mask;
        let low = (self.irq_counter & m).wrapping_sub(1) & m;
        self.irq_counter = (self.irq_counter & !m) | low;
        if low == m {
            self.irq_line = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ss() -> JalecoSs88006 {
        let prg: Vec<u8> = (0..16 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..64 * 0x400).map(|i| (i / 0x400) as u8).collect();
        JalecoSs88006::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn nibble_prg_banks_and_fixed_last() {
        let mut m = ss();
        m.cpu_write(0x8000, 0x5); // bank 0 low nibble
        m.cpu_write(0x8001, 0x0);
        m.cpu_write(0x8002, 0xA);
        m.cpu_write(0x9000, 0x3);
        m.cpu_write(0x9001, 0x0);
        assert_eq!(m.cpu_read(0x8000), 5);
        assert_eq!(m.cpu_read(0xA000), 10);
        assert_eq!(m.cpu_read(0xC000), 3);
        assert_eq!(m.cpu_read(0xE000), 15);
    }

    #[test]
    fn chr_registers_map_eight_slots() {
        let mut m = ss();
        // CHR reg 5 lives at $C002/$C003.
        m.cpu_write(0xC002, 0x4);
        m.cpu_write(0xC003, 0x2); // bank $24 = 36
        assert_eq!(m.ppu_read(0x1400), 36);
        m.cpu_write(0xA000, 0x7);
        assert_eq!(m.ppu_read(0x0000), 7);
    }

    #[test]
    fn prg_ram_enable_and_write_protect() {
        let mut m = ss();
        m.cpu_write(0x6000, 0x11);
        assert_eq!(m.prg_ram_read(0x6000), None);
        m.cpu_write(0x9002, 0x01); // enabled, read-only
        m.cpu_write(0x6000, 0x22);
        assert_eq!(m.prg_ram_read(0x6000), Some(0));
        m.cpu_write(0x9002, 0x03);
        m.cpu_write(0x6000, 0x33);
        assert_eq!(m.prg_ram_read(0x6000), Some(0x33));
    }

    #[test]
    fn irq_counter_width_modes() {
        let mut m = ss();
        // Reload $1234, 4-bit mode: only the low nibble (4) counts.
        for (i, n) in [4u8, 3, 2, 1].iter().enumerate() {
            m.cpu_write(0xE000 + i as u16, *n);
        }
        m.cpu_write(0xF000, 0);
        m.cpu_write(0xF001, 0x09);
        for _ in 0..4 {
            m.cpu_clock();
            assert!(!m.irq());
        }
        m.cpu_clock(); // 0 -> F underflow
        assert!(m.irq());
        assert_eq!(m.irq_counter & 0xFFF0, 0x1230, "upper bits untouched");
        m.cpu_write(0xF000, 0);
        assert!(!m.irq());
        // 16-bit mode counts the full value.
        m.cpu_write(0xF001, 0x01);
        for _ in 0..0x1234 {
            m.cpu_clock();
        }
        assert!(!m.irq());
        m.cpu_clock();
        assert!(m.irq());
    }

    #[test]
    fn mirroring_register() {
        let mut m = ss();
        m.cpu_write(0xF002, 0);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0xF002, 3);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
    }
}
