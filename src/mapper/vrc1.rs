use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Konami VRC1 (mapper 75).
///
/// PRG: three switchable 8KB banks at $8000/$A000/$C000 plus a fixed last
/// 8KB bank at $E000. CHR: two switchable 4KB banks at PPU $0000 and $1000.
///
/// Register map (writes; the low nibble holds the value unless noted):
/// - $8000: 8KB PRG bank at $8000
/// - $9000: bit0 mirroring (0=Vertical, 1=Horizontal); bit1 = high bit of the
///   CHR bank-0 selector; bit2 = high bit of the CHR bank-1 selector
/// - $A000: 8KB PRG bank at $A000
/// - $C000: 8KB PRG bank at $C000
/// - $E000: low 4 bits of the 4KB CHR bank at PPU $0000
/// - $F000: low 4 bits of the 4KB CHR bank at PPU $1000
///
/// Each CHR selector is a 5-bit value: the low nibble from $E000/$F000 plus
/// the matching high bit from $9000. No IRQ, no PRG RAM.
#[derive(Serialize, Deserialize)]
pub struct Vrc1 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    /// FourScreen comes from the cartridge header and overrides $9000 bit0.
    four_screen: bool,
    prg_banks: [u8; 3],
    // Low nibble of each 4KB CHR bank selector ($E000 / $F000).
    chr_low: [u8; 2],
    // High bits of each CHR selector, from $9000 bits 1 and 2.
    chr_high: [u8; 2],
}

impl Vrc1 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        let four_screen = mirroring == Mirroring::FourScreen;
        Vrc1 {
            prg,
            chr,
            chr_is_ram,
            mirroring,
            four_screen,
            prg_banks: [0; 3],
            chr_low: [0; 2],
            chr_high: [0; 2],
        }
    }

    /// Map a CPU address ($8000-$FFFF) to a PRG ROM offset (8KB banks).
    fn prg_offset(&self, addr: u16) -> usize {
        let banks = self.prg.len() / 0x2000;
        let bank = match (addr >> 13) & 3 {
            0 => self.prg_banks[0] as usize, // $8000
            1 => self.prg_banks[1] as usize, // $A000
            2 => self.prg_banks[2] as usize, // $C000
            _ => banks - 1,                  // $E000 fixed last
        };
        (bank % banks) * 0x2000 + (addr as usize & 0x1FFF)
    }

    /// Map a PPU address ($0000-$1FFF) to a CHR offset (4KB banks). The bank
    /// selector is the 5-bit value assembled from the low nibble and the
    /// $9000 high bit.
    fn chr_offset(&self, addr: u16) -> usize {
        let banks = (self.chr.len() / 0x1000).max(1);
        let half = (addr >> 12) as usize & 1;
        let bank = (self.chr_low[half] as usize) | ((self.chr_high[half] as usize) << 4);
        (bank % banks) * 0x1000 + (addr as usize & 0x0FFF)
    }
}

impl Mapper for Vrc1 {
    crate::impl_mapper_savestate!(chr);

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            let off = self.prg_offset(addr);
            self.prg[off]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr & 0xF000 {
            0x8000 => self.prg_banks[0] = val & 0x0F,
            0xA000 => self.prg_banks[1] = val & 0x0F,
            0xC000 => self.prg_banks[2] = val & 0x0F,
            0x9000 => {
                if !self.four_screen {
                    self.mirroring = if val & 0x01 != 0 {
                        Mirroring::Horizontal
                    } else {
                        Mirroring::Vertical
                    };
                }
                self.chr_high[0] = (val >> 1) & 1;
                self.chr_high[1] = (val >> 2) & 1;
            }
            0xE000 => self.chr_low[0] = val & 0x0F,
            0xF000 => self.chr_low[1] = val & 0x0F,
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let off = self.chr_offset(addr);
        self.chr[off]
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

    /// 8 PRG banks (64KB), 32 CHR 4KB banks (128KB). Each PRG 8KB bank is
    /// filled with its bank index; each CHR byte encodes its 4KB bank index.
    fn vrc1() -> Vrc1 {
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..32 * 0x1000).map(|i| (i / 0x1000) as u8).collect();
        Vrc1::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_three_switchable_plus_fixed_last() {
        let mut m = vrc1();
        // Defaults: $8000/$A000/$C000 -> bank 0; $E000 -> last (7).
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xA000), 0);
        assert_eq!(m.cpu_read(0xC000), 0);
        assert_eq!(m.cpu_read(0xE000), 7);

        m.cpu_write(0x8000, 3);
        m.cpu_write(0xA000, 5);
        m.cpu_write(0xC000, 6);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xA000), 5);
        assert_eq!(m.cpu_read(0xC000), 6);
        // $E000 stays fixed to the last bank regardless.
        assert_eq!(m.cpu_read(0xE000), 7);
    }

    #[test]
    fn chr_4kb_banking_with_high_bit() {
        let mut m = vrc1();
        // Low nibbles only: $0000 -> bank 5, $1000 -> bank 9.
        m.cpu_write(0xE000, 0x05);
        m.cpu_write(0xF000, 0x09);
        assert_eq!(m.ppu_read(0x0000), 5);
        assert_eq!(m.ppu_read(0x0FFF), 5);
        assert_eq!(m.ppu_read(0x1000), 9);
        assert_eq!(m.ppu_read(0x1FFF), 9);

        // $9000 bit1 sets CHR-0 high bit (+16), bit2 sets CHR-1 high bit.
        // bit1=1, bit2=1 -> 5|16 = 21, 9|16 = 25.
        m.cpu_write(0x9000, 0b110);
        assert_eq!(m.ppu_read(0x0000), 21);
        assert_eq!(m.ppu_read(0x1000), 25);

        // Only CHR-1 high bit set: CHR-0 reverts to 5, CHR-1 = 25.
        m.cpu_write(0x9000, 0b100);
        assert_eq!(m.ppu_read(0x0000), 5);
        assert_eq!(m.ppu_read(0x1000), 25);
    }

    #[test]
    fn mirroring_decode() {
        let mut m = vrc1();
        m.cpu_write(0x9000, 0x00);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0x9000, 0x01);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        // High CHR bits don't disturb the mirroring bit.
        m.cpu_write(0x9000, 0b110);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0x9000, 0b111);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn four_screen_overrides_mirroring_reg() {
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..32 * 0x1000).map(|i| (i / 0x1000) as u8).collect();
        let mut m = Vrc1::new(prg, chr, Mirroring::FourScreen);
        m.cpu_write(0x9000, 0x01);
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
    }
}
