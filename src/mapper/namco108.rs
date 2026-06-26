use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Namco 108 / DxROM (mapper 206): the bank-switching ancestor of the MMC3.
/// It uses the same $8000/$8001 select/data register pair and the same fixed
/// banking layout as MMC3's mode 0, but drops the PRG/CHR mode bits, the
/// scanline IRQ, the mirroring register (hardwired by the board) and PRG RAM.
/// Bank numbers are narrower: CHR registers are 6-bit, PRG registers 4-bit.
#[derive(Clone, Serialize, Deserialize)]
pub struct Namco108 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    bank_select: u8,
    bank_regs: [u8; 8],
}

impl Namco108 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Namco108 {
            prg,
            chr,
            chr_is_ram,
            mirroring,
            bank_select: 0,
            bank_regs: [0; 8],
        }
    }

    /// Map a CPU address ($8000-$FFFF) to a PRG ROM offset (8KB banks). R6/R7
    /// switch $8000/$A000; $C000/$E000 are fixed to the last two banks.
    fn prg_offset(&self, addr: u16) -> usize {
        let banks = self.prg.len() / 0x2000;
        let bank = match (addr >> 13) & 3 {
            0 => self.bank_regs[6] as usize % banks,
            1 => self.bank_regs[7] as usize % banks,
            2 => banks - 2,
            _ => banks - 1,
        };
        bank * 0x2000 + (addr as usize & 0x1FFF)
    }

    /// Map a PPU address ($0000-$1FFF) to a CHR offset (1KB banks). R0/R1 are
    /// 2KB banks at $0000/$0800; R2-R5 are 1KB banks at $1000-$1C00.
    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x400;
        let bank = match addr >> 10 {
            0 => self.bank_regs[0] as usize & !1,
            1 => self.bank_regs[0] as usize | 1,
            2 => self.bank_regs[1] as usize & !1,
            3 => self.bank_regs[1] as usize | 1,
            k => self.bank_regs[k as usize - 2] as usize,
        } % banks;
        bank * 0x400 + (addr as usize & 0x3FF)
    }
}

impl Mapper for Namco108 {
    crate::impl_mapper_savestate!(chr_is_ram = chr_is_ram);
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            if addr & 1 == 0 {
                self.bank_select = val & 7;
            } else {
                // CHR registers (0-5) are 6-bit; PRG registers (6-7) 4-bit.
                let masked = if self.bank_select < 6 {
                    val & 0x3F
                } else {
                    val & 0x0F
                };
                self.bank_regs[self.bank_select as usize] = masked;
            }
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

    fn namco108() -> Namco108 {
        // 4 PRG banks (32KB), 8 CHR banks (8KB); each byte = its bank index.
        let prg: Vec<u8> = (0..4 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Namco108::new(prg, chr, Mirroring::Horizontal)
    }

    #[test]
    fn prg_fixed_and_switchable() {
        let mut m = namco108();
        // $C000/$E000 always last two banks.
        assert_eq!(m.cpu_read(0xC000), 2);
        assert_eq!(m.cpu_read(0xE000), 3);
        m.cpu_write(0x8000, 6); // select R6
        m.cpu_write(0x8001, 1);
        m.cpu_write(0x8000, 7); // select R7
        m.cpu_write(0x8001, 0);
        assert_eq!(m.cpu_read(0x8000), 1);
        assert_eq!(m.cpu_read(0xA000), 0);
    }

    #[test]
    fn chr_banking_no_inversion() {
        let mut m = namco108();
        m.cpu_write(0x8000, 0); // R0: 2KB at $0000 (low bit ignored)
        m.cpu_write(0x8001, 4);
        m.cpu_write(0x8000, 2); // R2: 1KB at $1000
        m.cpu_write(0x8001, 7);
        assert_eq!(m.ppu_read(0x0000), 4);
        assert_eq!(m.ppu_read(0x0400), 5);
        assert_eq!(m.ppu_read(0x1000), 7);
        // No CHR mode bit: bit 7 of the select register is masked away.
        m.cpu_write(0x8000, 0x80);
        assert_eq!(m.ppu_read(0x0000), 4);
    }

    #[test]
    fn chr_register_is_6_bit() {
        let mut m = namco108();
        m.cpu_write(0x8000, 2); // R2
        m.cpu_write(0x8001, 0xFF); // masked to 0x3F
        // 8 banks => 0x3F % 8 = 7.
        assert_eq!(m.ppu_read(0x1000), 7);
    }
}
