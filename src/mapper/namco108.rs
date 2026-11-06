use super::{Mapper, Mirroring, NtTarget, mirror_nt};
use serde::{Deserialize, Serialize};

/// Namco 108 / DxROM (mapper 206): the bank-switching ancestor of the MMC3.
/// It uses the same $8000/$8001 select/data register pair and the same fixed
/// banking layout as MMC3's mode 0, but drops the PRG/CHR mode bits, the
/// scanline IRQ, the mirroring register (hardwired by the board) and PRG RAM.
/// Bank numbers are narrower: CHR registers are 6-bit, PRG registers 4-bit.
///
/// [`Namco108Board`] variants cover the boards that rewire its CHR lines.
#[derive(Serialize, Deserialize)]
pub struct Namco108 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    #[serde(default)]
    board: Namco108Board,
    mirroring: Mirroring,
    bank_select: u8,
    bank_regs: [u8; 8],
}

/// Namco 108 boards with rewired CHR address lines.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Namco108Board {
    /// Mapper 206: DxROM / Namco 108 as-is.
    #[default]
    Standard,
    /// Mapper 76, Namco 3446 (Megami Tensei): R2-R5 select four 2KB CHR
    /// banks; R0/R1 do nothing. Up to 128KB CHR.
    Namco3446,
    /// Mapper 88, Namco 3433: CHR A16 is tied to PPU A12, so $0000-$0FFF
    /// sees the low 64KB of CHR and $1000-$1FFF the high 64KB.
    Namco3433,
    /// Mapper 154, Namco 3453 (Devil Man): mapper 88 plus a one-screen
    /// mirroring select in bit 6 of every $8000-$FFFF write.
    Namco3453,
    /// Mapper 95, Namco 3425 (Dragon Buster): bit 5 of R0 / R1 drives
    /// CIRAM A10 for the $2000-$27FF / $2800-$2FFF nametables.
    Namco3425,
}

impl Namco108 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        Self::with_board(Namco108Board::Standard, prg, chr, mirroring)
    }

    pub fn with_board(
        board: Namco108Board,
        prg: Vec<u8>,
        chr: Vec<u8>,
        mirroring: Mirroring,
    ) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        // Devil Man powers on showing nametable page 0.
        let mirroring = if board == Namco108Board::Namco3453 {
            Mirroring::SingleScreenLo
        } else {
            mirroring
        };
        Namco108 {
            prg,
            chr,
            chr_is_ram,
            board,
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
        if self.board == Namco108Board::Namco3446 {
            // Four 2KB banks from R2-R5.
            let reg = self.bank_regs[2 + (addr as usize >> 11)] as usize;
            let bank = (reg * 2 + ((addr as usize >> 10) & 1)) % banks;
            return bank * 0x400 + (addr as usize & 0x3FF);
        }
        let bank = match addr >> 10 {
            0 => self.bank_regs[0] as usize & !1,
            1 => self.bank_regs[0] as usize | 1,
            2 => self.bank_regs[1] as usize & !1,
            3 => self.bank_regs[1] as usize | 1,
            k => self.bank_regs[k as usize - 2] as usize,
        };
        let bank = match self.board {
            // CHR A16 follows PPU A12.
            Namco108Board::Namco3433 | Namco108Board::Namco3453 => {
                (bank & 0x3F) | ((addr as usize & 0x1000) >> 6)
            }
            // Bit 5 of the 2KB registers goes to CIRAM A10, not CHR.
            Namco108Board::Namco3425 => bank & 0x1F,
            _ => bank,
        } % banks;
        bank * 0x400 + (addr as usize & 0x3FF)
    }
}

impl Mapper for Namco108 {
    crate::impl_mapper_savestate!(chr);

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
        if addr >= 0x8000 {
            if self.board == Namco108Board::Namco3453 {
                self.mirroring = if val & 0x40 != 0 {
                    Mirroring::SingleScreenHi
                } else {
                    Mirroring::SingleScreenLo
                };
            }
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

    fn nt_target(&mut self, addr: u16) -> NtTarget {
        if self.board != Namco108Board::Namco3425 || self.mirroring == Mirroring::FourScreen {
            return NtTarget::Ciram(mirror_nt(self.mirroring, addr));
        }
        let reg = self.bank_regs[((addr >> 11) & 1) as usize];
        let a10 = ((reg >> 5) & 1) as u16;
        NtTarget::Ciram((a10 << 10) | (addr & 0x3FF))
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

    /// 8 PRG banks, 128KB CHR; bytes = their 1KB CHR page index.
    fn board(b: Namco108Board) -> Namco108 {
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..128 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Namco108::with_board(b, prg, chr, Mirroring::Vertical)
    }

    fn set_reg(m: &mut Namco108, r: u8, v: u8) {
        m.cpu_write(0x8000, r);
        m.cpu_write(0x8001, v);
    }

    #[test]
    fn namco3446_uses_four_2kb_banks() {
        let mut m = board(Namco108Board::Namco3446);
        for (r, v) in [(2, 3), (3, 10), (4, 0x20), (5, 0x3F)] {
            set_reg(&mut m, r, v);
        }
        set_reg(&mut m, 0, 7); // R0 is unused on this board
        assert_eq!(m.ppu_read(0x0000), 6);
        assert_eq!(m.ppu_read(0x0400), 7);
        assert_eq!(m.ppu_read(0x0800), 20);
        assert_eq!(m.ppu_read(0x1000), 64);
        assert_eq!(m.ppu_read(0x1C00), 127);
    }

    #[test]
    fn namco3433_ties_chr_a16_to_a12() {
        let mut m = board(Namco108Board::Namco3433);
        set_reg(&mut m, 0, 4);
        set_reg(&mut m, 2, 4);
        assert_eq!(m.ppu_read(0x0000), 4, "low 64KB below $1000");
        assert_eq!(m.ppu_read(0x1000), 0x44, "high 64KB from $1000");
    }

    #[test]
    fn namco3453_one_screen_select() {
        let mut m = board(Namco108Board::Namco3453);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        m.cpu_write(0xC000, 0x40);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
        m.cpu_write(0x8000, 0x00);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        set_reg(&mut m, 5, 1);
        assert_eq!(m.ppu_read(0x1C00), 0x41);
    }

    #[test]
    fn namco3425_nametables_follow_chr_bit5() {
        let mut m = board(Namco108Board::Namco3425);
        set_reg(&mut m, 0, 0x20 | 2);
        set_reg(&mut m, 1, 4);
        assert_eq!(m.nt_target(0x2000), NtTarget::Ciram(0x400));
        assert_eq!(m.nt_target(0x2400), NtTarget::Ciram(0x400));
        assert_eq!(m.nt_target(0x2800), NtTarget::Ciram(0x000));
        assert_eq!(m.nt_target(0x2C05), NtTarget::Ciram(0x005));
        // Bit 5 never reaches the CHR address lines.
        assert_eq!(m.ppu_read(0x0000), 2);
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
