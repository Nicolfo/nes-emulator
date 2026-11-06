use super::discrete::{bank_index, chr_or_ram};
use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// How the board wires the nametables, from the header's mirroring bits
/// (flags 6 bits 0 and 3) and the submapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum NtMode {
    Fixed(Mirroring),
    /// Register bit 7 picks the single-screen page.
    OneScreen,
    /// Submapper 3: register bit 7 picks horizontal (0) / vertical (1).
    Switchable,
}

/// UNROM 512 (mapper 30), the RetroUSB/InfiniteNESLives homebrew board:
/// register `[MCCP PPPP]` - 16KB PRG bank at $8000 (last bank fixed at
/// $C000), 8KB CHR RAM bank out of 32KB, and bit 7 the one-screen page on
/// single-screen boards. On the self-flashable variant (battery bit set) the
/// register only answers at $C000-$FFFF; $8000-$BFFF are flash commands,
/// which are accepted and ignored (no self-saving).
#[derive(Serialize, Deserialize)]
pub struct Unrom512 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    flash: bool,
    nt: NtMode,
    reg: u8,
}

impl Unrom512 {
    /// `header_nt` is flags 6 & $09 (bit 0 vertical, bit 3 four-screen
    /// pad); `battery` marks the self-flashable board.
    pub fn new(submapper: u8, header_nt: u8, battery: bool, prg: Vec<u8>, chr: Vec<u8>) -> Self {
        let (chr, chr_is_ram) = chr_or_ram(chr, 0x8000);
        let nt = if submapper == 3 {
            NtMode::Switchable
        } else {
            match header_nt & 0x09 {
                0x00 => NtMode::Fixed(Mirroring::Horizontal),
                0x01 => NtMode::Fixed(Mirroring::Vertical),
                0x08 => NtMode::OneScreen,
                _ => NtMode::Fixed(Mirroring::FourScreen),
            }
        };
        Unrom512 {
            prg,
            chr,
            chr_is_ram,
            flash: battery,
            nt,
            reg: 0,
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        bank_index(
            self.chr.len(),
            0x2000,
            ((self.reg >> 5) & 3) as usize,
            addr as usize,
        )
    }
}

impl Mapper for Unrom512 {
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
        let len = self.prg.len();
        let bank = if addr < 0xC000 {
            (self.reg & 0x1F) as usize
        } else {
            len / 0x4000 - 1
        };
        self.prg[bank_index(len, 0x4000, bank, addr as usize)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        let reg_range = if self.flash { 0xC000 } else { 0x8000 };
        if addr >= reg_range {
            self.reg = val;
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
        let bit7 = self.reg & 0x80 != 0;
        match self.nt {
            NtMode::Fixed(m) => m,
            NtMode::OneScreen if bit7 => Mirroring::SingleScreenHi,
            NtMode::OneScreen => Mirroring::SingleScreenLo,
            NtMode::Switchable if bit7 => Mirroring::Vertical,
            NtMode::Switchable => Mirroring::Horizontal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::discrete::tests::banks;

    #[test]
    fn prg_and_chr_ram_banks() {
        let mut m = Unrom512::new(0, 0x01, false, banks(32, 0x4000), vec![]);
        m.cpu_write(0x8000, 0b0110_0101);
        assert_eq!(m.cpu_read(0x8000), 5);
        assert_eq!(m.cpu_read(0xFFFF), 31, "last bank fixed");
        m.ppu_write(0x0010, 0xAB); // CHR bank 3
        m.cpu_write(0x8000, 0);
        assert_eq!(m.ppu_read(0x0010), 0);
        m.cpu_write(0x8000, 0x60);
        assert_eq!(m.ppu_read(0x0010), 0xAB);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn one_screen_and_flash_variants() {
        let mut m = Unrom512::new(0, 0x08, true, banks(32, 0x4000), vec![]);
        m.cpu_write(0x8000, 0x83); // flash command space: ignored
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        m.cpu_write(0xC000, 0x83);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);

        let mut s3 = Unrom512::new(3, 0x00, false, banks(2, 0x4000), vec![]);
        s3.cpu_write(0x8000, 0x80);
        assert_eq!(s3.mirroring(), Mirroring::Vertical);
    }
}
