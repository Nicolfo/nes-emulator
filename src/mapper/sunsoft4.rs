use super::{Mapper, Mirroring, NtTarget, mirror_nt};
use serde::{Deserialize, Serialize};

/// Sunsoft-4 (mapper 68, After Burner, Maharaja): 16KB PRG banking with a
/// fixed last bank, four 2KB CHR banks, optional 8KB PRG RAM, and the
/// defining feature — two CHR-ROM banks that can be mapped into nametable
/// space in place of the console's CIRAM.
#[derive(Serialize, Deserialize)]
pub struct Sunsoft4 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    #[serde(with = "crate::savestate::byte_array")]
    prg_ram: [u8; 0x2000],
    // $8000/$9000/$A000/$B000: 2KB CHR bank at $0000/$0800/$1000/$1800.
    chr_banks: [u8; 4],
    // $C000/$D000: 1KB CHR-ROM bank numbers used as nametables (bits 6-0)
    // when CHR-ROM nametable mode is enabled. Index 0 backs the logical
    // "low" nametable, index 1 the "high" one.
    nt_banks: [u8; 2],
    // $E000 bits 1-0: mirroring mode.
    mirroring: Mirroring,
    fourscreen: bool,
    // $E000 bit 4: 0 = CIRAM nametables, 1 = CHR-ROM nametables.
    chr_rom_nt: bool,
    // $F000 bits 3-0: 16KB PRG bank at $8000-$BFFF.
    prg_bank: u8,
    // $F000 bit 4: enable 8KB PRG RAM at $6000-$7FFF.
    prg_ram_enabled: bool,
}

impl Sunsoft4 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        Sunsoft4 {
            prg,
            chr,
            prg_ram: [0; 0x2000],
            chr_banks: [0; 4],
            nt_banks: [0; 2],
            mirroring,
            fourscreen: mirroring == Mirroring::FourScreen,
            chr_rom_nt: false,
            prg_bank: 0,
            prg_ram_enabled: false,
        }
    }

    /// Map a CPU address ($8000-$FFFF) to a PRG ROM offset (16KB banks).
    /// $8000-$BFFF is switchable; $C000-$FFFF is fixed to the last bank.
    fn prg_offset(&self, addr: u16) -> usize {
        let banks = (self.prg.len() / 0x4000).max(1);
        let bank = if addr < 0xC000 {
            self.prg_bank as usize % banks
        } else {
            banks - 1
        };
        bank * 0x4000 + (addr as usize & 0x3FFF)
    }

    /// Map a PPU pattern address ($0000-$1FFF) to a CHR offset (2KB banks).
    fn chr_offset(&self, addr: u16) -> usize {
        let banks = (self.chr.len() / 0x800).max(1);
        let bank = self.chr_banks[((addr >> 11) & 3) as usize] as usize % banks;
        bank * 0x800 + (addr as usize & 0x7FF)
    }

    /// Read a CHR-ROM nametable byte for a $2000-$2FFF address.
    ///
    /// VERIFY-AGAINST-ROM: this is the part most likely to need checking on
    /// real hardware. The $E000 mirroring mode decides which of the two 1KB
    /// CHR-ROM banks ($C000 -> nt_banks[0] "low", $D000 -> nt_banks[1]
    /// "high") backs each of the four logical nametables. We reuse
    /// `mirror_nt` to derive the low/high page (bit 10 of the resulting
    /// CIRAM offset), so the orientation exactly matches the CIRAM layout:
    ///   Vertical:  L H L H   Horizontal: L L H H
    ///   SS-Lo: all L         SS-Hi: all H
    /// Only bits 6-0 of the bank register are used (bit 7 ignored), so
    /// nametables come from the final 128KB of CHR.
    fn chr_nt_offset(&self, addr: u16) -> usize {
        let page = (mirror_nt(self.mirroring, addr) >> 10) & 1;
        // Only D6-D0 are used; D7 is ignored and treated as 1, so a nametable
        // always comes from the last 128KB of CHR ROM.
        let bank = 0x80 | (self.nt_banks[page as usize] & 0x7F) as usize;
        let banks = (self.chr.len() / 0x400).max(1);
        (bank % banks) * 0x400 + (addr as usize & 0x3FF)
    }
}

impl Mapper for Sunsoft4 {
    crate::impl_mapper_savestate!();

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr & 0xF000 {
            0x6000 | 0x7000 => {
                if self.prg_ram_enabled {
                    self.prg_ram[(addr & 0x1FFF) as usize] = val;
                }
            }
            0x8000 => self.chr_banks[0] = val,
            0x9000 => self.chr_banks[1] = val,
            0xA000 => self.chr_banks[2] = val,
            0xB000 => self.chr_banks[3] = val,
            0xC000 => self.nt_banks[0] = val,
            0xD000 => self.nt_banks[1] = val,
            0xE000 => {
                if !self.fourscreen {
                    self.mirroring = match val & 3 {
                        0 => Mirroring::Vertical,
                        1 => Mirroring::Horizontal,
                        2 => Mirroring::SingleScreenLo,
                        _ => Mirroring::SingleScreenHi,
                    };
                }
                self.chr_rom_nt = val & 0x10 != 0;
            }
            0xF000 => {
                self.prg_bank = val & 0x0F;
                self.prg_ram_enabled = val & 0x10 != 0;
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x2000 {
            self.chr[self.chr_offset(addr)]
        } else {
            // CHR-ROM nametable fetch ($2000-$2FFF). Only reached when
            // nt_target returned Cart, i.e. chr_rom_nt is set.
            self.chr[self.chr_nt_offset(addr)]
        }
    }

    fn ppu_write(&mut self, _addr: u16, _val: u8) {
        // CHR is ROM and the CHR-ROM nametables are ROM too; CIRAM writes
        // are routed via NtTarget::Ciram instead, so nothing to do here.
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn nt_target(&mut self, addr: u16) -> NtTarget {
        // A four-screen board always uses its own RAM. Otherwise, when
        // CHR-ROM nametables are enabled the cartridge serves the byte;
        // when disabled fall back to normal CIRAM mirroring.
        if self.chr_rom_nt && !self.fourscreen {
            NtTarget::Cart
        } else {
            NtTarget::Ciram(mirror_nt(self.mirroring, addr))
        }
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        if self.prg_ram_enabled {
            Some(self.prg_ram[(addr & 0x1FFF) as usize])
        } else {
            None // open bus
        }
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

    fn sunsoft4() -> Sunsoft4 {
        // 4 PRG banks (64KB): each 16KB bank filled with its index.
        let prg: Vec<u8> = (0..4 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        // 8 CHR 2KB banks (16KB): each 2KB bank filled with its index.
        let chr: Vec<u8> = (0..8 * 0x800).map(|i| (i / 0x800) as u8).collect();
        Sunsoft4::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_switch_and_fixed() {
        let mut m = sunsoft4();
        // Default switchable bank 0; fixed bank = last (3).
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xC000), 3);
        assert_eq!(m.cpu_read(0xFFFF), 3);
        // Switch $8000-$BFFF to bank 2.
        m.cpu_write(0xF000, 2);
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xBFFF), 2);
        // Fixed bank unaffected.
        assert_eq!(m.cpu_read(0xC000), 3);
        // Only low 4 bits select the bank; bit 4 is the RAM enable.
        m.cpu_write(0xF000, 0x11);
        assert_eq!(m.cpu_read(0x8000), 1);
    }

    #[test]
    fn prg_ram_enable_gating() {
        let mut m = sunsoft4();
        // Disabled at power-on: writes ignored, reads open bus.
        m.cpu_write(0x6000, 0xAA);
        assert_eq!(m.prg_ram_read(0x6000), None);
        // Enable via $F000 bit 4.
        m.cpu_write(0xF000, 0x10);
        m.cpu_write(0x6123, 0xBB);
        assert_eq!(m.prg_ram_read(0x6123), Some(0xBB));
        // Disable again -> open bus, contents retained for battery save.
        m.cpu_write(0xF000, 0x00);
        assert_eq!(m.prg_ram_read(0x6123), None);
        assert_eq!(m.prg_ram().unwrap()[0x123], 0xBB);
    }

    #[test]
    fn chr_2kb_banking() {
        let mut m = sunsoft4();
        m.cpu_write(0x8000, 1); // $0000-$07FF -> bank 1
        m.cpu_write(0x9000, 3); // $0800-$0FFF -> bank 3
        m.cpu_write(0xA000, 5); // $1000-$17FF -> bank 5
        m.cpu_write(0xB000, 7); // $1800-$1FFF -> bank 7
        assert_eq!(m.ppu_read(0x0000), 1);
        assert_eq!(m.ppu_read(0x07FF), 1);
        assert_eq!(m.ppu_read(0x0800), 3);
        assert_eq!(m.ppu_read(0x1000), 5);
        assert_eq!(m.ppu_read(0x1FFF), 7);
    }

    #[test]
    fn mirroring_decode() {
        let mut m = sunsoft4();
        m.cpu_write(0xE000, 0);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0xE000, 1);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0xE000, 2);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        m.cpu_write(0xE000, 3);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
    }

    #[test]
    fn fourscreen_ignores_mirroring_and_chr_rom_nt() {
        let prg: Vec<u8> = (0..4 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x800).map(|i| (i / 0x800) as u8).collect();
        let mut m = Sunsoft4::new(prg, chr, Mirroring::FourScreen);
        // Mirroring register is ignored on a four-screen board.
        m.cpu_write(0xE000, 0x11);
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
        // CHR-ROM nametables stay disabled (routes to CIRAM/console RAM).
        assert_eq!(
            m.nt_target(0x2000),
            NtTarget::Ciram(mirror_nt(Mirroring::FourScreen, 0x2000))
        );
    }

    #[test]
    fn nt_target_cart_when_chr_rom_nt_enabled() {
        let mut m = sunsoft4();
        m.cpu_write(0xE000, 0); // vertical, CHR-ROM NT off
        // Off -> normal CIRAM mirroring.
        assert_eq!(m.nt_target(0x2000), NtTarget::Ciram(0x000));
        assert_eq!(m.nt_target(0x2400), NtTarget::Ciram(0x400));
        // Enable CHR-ROM nametables (bit 4) -> cartridge serves the byte.
        m.cpu_write(0xE000, 0x10);
        assert_eq!(m.nt_target(0x2000), NtTarget::Cart);
        assert_eq!(m.nt_target(0x2C00), NtTarget::Cart);
    }

    #[test]
    fn chr_rom_nametable_fetch() {
        let mut m = sunsoft4();
        // Vertical + CHR-ROM nametables on.
        m.cpu_write(0xE000, 0x10);
        m.cpu_write(0xC000, 2); // low NT  -> 1KB CHR bank 2
        m.cpu_write(0xD000, 5); // high NT -> 1KB CHR bank 5
        // CHR byte = (offset / 0x800) i.e. 2KB-bank index; bank 2 of 1KB is
        // within 2KB-bank 1, bank 5 within 2KB-bank 2.
        // Vertical: $2000 = low, $2400 = high.
        assert_eq!(m.ppu_read(0x2000), 1); // 1KB bank 2 -> 2KB bank 1
        assert_eq!(m.ppu_read(0x2400), 2); // 1KB bank 5 -> 2KB bank 2
        // Single-screen-low: both map to the low bank.
        m.cpu_write(0xE000, 0x12);
        assert_eq!(m.ppu_read(0x2000), 1);
        assert_eq!(m.ppu_read(0x2400), 1);
    }
}
