use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Mapper 78 (Irem 74HC161) - Holy Diver / Cosmo Carrier.
///
/// Single 8-bit write port: any write to $8000-$FFFF latches one byte:
/// ```text
/// 7  bit  0
/// C C C C M P P P
/// | | | | | | | |
/// | | | | | +-+-+- bits 0-2: 16KB PRG bank at $8000 (switchable)
/// | | | | +------- bit 3 (M): mirroring control
/// +-+-+-+--------- bits 4-7: 8KB CHR bank (switchable)
/// ```
/// PRG: $8000-$BFFF switchable (bits 0-2), $C000-$FFFF fixed to the last 16KB
/// bank (UxROM-like). CHR: one 8KB switchable bank (bits 4-7).
///
/// This is submapper 3 (Holy Diver): bit 3 selects Horizontal/Vertical
/// mirroring. (Submapper 1 / Cosmo Carrier uses single-screen instead.)
#[derive(Serialize, Deserialize)]
pub struct HolyDiver {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
    /// Header-supplied four-screen flag; when set, the mapper's own mirroring
    /// control is bypassed and never overwritten.
    four_screen: bool,
}

impl HolyDiver {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        let four_screen = mirroring == Mirroring::FourScreen;
        HolyDiver {
            prg,
            chr,
            chr_is_ram,
            prg_bank: 0,
            chr_bank: 0,
            mirroring,
            four_screen,
        }
    }
}

impl Mapper for HolyDiver {
    crate::impl_mapper_savestate!(chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        let banks = self.prg.len() / 0x4000;
        match addr {
            0x8000..=0xBFFF => {
                self.prg[(self.prg_bank as usize % banks) * 0x4000 + (addr as usize & 0x3FFF)]
            }
            0xC000..=0xFFFF => self.prg[(banks - 1) * 0x4000 + (addr as usize & 0x3FFF)],
            _ => 0,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr >= 0x8000 {
            self.prg_bank = val & 0x07;
            self.chr_bank = (val >> 4) & 0x0F;
            // Submapper 3 (Holy Diver): bit 3 toggles H/V mirroring.
            // NOTE: this polarity may be flipped by the caller after testing
            // against the M78.3 ROM. Kept isolated to this single expression
            // for an easy flip. Four-screen boards bypass mapper mirroring.
            if !self.four_screen {
                self.mirroring = if val & 0x08 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let banks = self.chr.len() / 0x2000;
        self.chr[(self.chr_bank as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.chr_is_ram {
            self.chr[(addr as usize) & 0x1FFF] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holydiver() -> HolyDiver {
        // 8 PRG banks (128KB): each PRG byte = its bank index.
        // 8 CHR banks (64KB): each CHR byte = its bank index.
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        HolyDiver::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_switchable_8000_fixed_c000() {
        let mut m = holydiver();
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xC000), 7); // last bank, fixed
        m.cpu_write(0x8000, 0x02); // PRG bank bits 0-2 = 2
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xC000), 7); // still fixed to last
    }

    #[test]
    fn prg_bank_uses_only_low_three_bits() {
        let mut m = holydiver();
        // CHR/mirroring bits set, PRG field = 3.
        m.cpu_write(0x8000, 0xFB); // 1111_1011 -> prg = 3
        assert_eq!(m.cpu_read(0x8000), 3);
    }

    #[test]
    fn chr_switchable() {
        let mut m = holydiver();
        assert_eq!(m.ppu_read(0x0000), 0);
        m.cpu_write(0x8000, 0x30); // CHR bank = bits 4-7 = 3
        assert_eq!(m.ppu_read(0x0000), 3);
        // CHR field wraps modulo bank count (8 banks here, value 0xA -> 2).
        m.cpu_write(0x8000, 0xA0);
        assert_eq!(m.ppu_read(0x0000), 2);
    }

    #[test]
    fn mirroring_toggle_both_states() {
        let mut m = holydiver();
        m.cpu_write(0x8000, 0x00); // bit 3 clear
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0x8000, 0x08); // bit 3 set
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0x8000, 0x00); // back to clear
        assert_eq!(m.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn four_screen_never_overwritten() {
        let prg: Vec<u8> = (0..2 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = vec![0; 0x2000];
        let mut m = HolyDiver::new(prg, chr, Mirroring::FourScreen);
        m.cpu_write(0x8000, 0x08); // would set Horizontal, but four-screen wins
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
        m.cpu_write(0x8000, 0x00);
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
    }

    #[test]
    fn chr_ram_fallback_rw() {
        let prg: Vec<u8> = (0..2 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let mut m = HolyDiver::new(prg, vec![], Mirroring::Vertical);
        m.ppu_write(0x1234, 0xAB);
        assert_eq!(m.ppu_read(0x1234), 0xAB);
    }
}
