use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// MMC1 (mapper 1): registers loaded one bit at a time through a 5-bit
/// shift register on writes to $8000-$FFFF. Control register selects
/// mirroring (including single-screen), PRG mode (32KB / fix-first /
/// fix-last) and CHR mode (8KB / two 4KB banks).
///
/// WRAM can be disabled by PRG bank register bit 4, or — on SNROM boards
/// (8KB CHR RAM, <512KB PRG) — by CHR bank register bit 4. Disabled WRAM
/// reads as open bus and ignores writes.
///
/// Not emulated: the consecutive-cycle write-ignore quirk (only matters for
/// games doing read-modify-write stores to $8000+).
#[derive(Serialize, Deserialize)]
pub struct Mmc1 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    #[serde(with = "crate::savestate::byte_array")]
    prg_ram: [u8; 0x2000],
    shift: u8,
    shift_count: u8,
    control: u8,
    chr_bank0: u8,
    chr_bank1: u8,
    prg_bank: u8,
}

impl Mmc1 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Mmc1 {
            prg,
            chr,
            chr_is_ram,
            prg_ram: [0; 0x2000],
            shift: 0,
            shift_count: 0,
            // Power-on: PRG mode 3 (fix last bank at $C000) so the reset
            // vector in the last bank is reachable.
            control: 0x0C,
            chr_bank0: 0,
            chr_bank1: 0,
            prg_bank: 0,
        }
    }

    /// Map a CPU address ($8000-$FFFF) to a PRG ROM offset (16KB banks).
    fn prg_offset(&self, addr: u16) -> usize {
        // SUROM (512KB): CHR bank register bit 4 selects the 256KB half;
        // banking below operates within that half. Real hardware uses
        // whichever CHR register the PPU last addressed, but SUROM games
        // set both identically.
        let banks = (self.prg.len() / 0x4000).min(16);
        let bank = (self.prg_bank & 0x0F) as usize;
        let off = match (self.control >> 2) & 3 {
            0 | 1 => ((bank & !1) % banks) * 0x4000 + (addr as usize & 0x7FFF),
            2 => {
                if addr < 0xC000 {
                    addr as usize & 0x3FFF
                } else {
                    (bank % banks) * 0x4000 + (addr as usize & 0x3FFF)
                }
            }
            _ => {
                if addr < 0xC000 {
                    (bank % banks) * 0x4000 + (addr as usize & 0x3FFF)
                } else {
                    (banks - 1) * 0x4000 + (addr as usize & 0x3FFF)
                }
            }
        };
        let half = if self.prg.len() >= 0x80000 {
            (self.chr_bank0 as usize & 0x10) << 14
        } else {
            0
        };
        half | off
    }

    /// WRAM disable: PRG bank register bit 4 on all boards; on SNROM
    /// (8KB CHR RAM, <512KB PRG — bit 4 isn't a CHR bank or SUROM half
    /// select there) the CHR bank registers' bit 4 too.
    fn wram_disabled(&self) -> bool {
        if self.prg_bank & 0x10 != 0 {
            return true;
        }
        let snrom = self.chr_is_ram && self.chr.len() == 0x2000 && self.prg.len() < 0x80000;
        let chr_bits = if self.control & 0x10 != 0 {
            self.chr_bank0 | self.chr_bank1
        } else {
            self.chr_bank0
        };
        snrom && chr_bits & 0x10 != 0
    }

    /// Map a PPU address ($0000-$1FFF) to a CHR offset (4KB banks).
    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x1000;
        if self.control & 0x10 != 0 {
            // Two independent 4KB banks.
            let reg = if addr < 0x1000 {
                self.chr_bank0
            } else {
                self.chr_bank1
            };
            (reg as usize % banks) * 0x1000 + (addr as usize & 0x0FFF)
        } else {
            // 8KB mode: low bit of chr_bank0 ignored.
            ((self.chr_bank0 as usize & !1) % banks) * 0x1000 + (addr as usize & 0x1FFF)
        }
    }
}

impl Mapper for Mmc1 {
    crate::impl_mapper_savestate!();
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x6000..=0x7FFF => {
                if !self.wram_disabled() {
                    self.prg_ram[(addr & 0x1FFF) as usize] = val;
                }
            }
            0x8000..=0xFFFF => {
                if val & 0x80 != 0 {
                    // Reset: clear shift register and force PRG mode 3.
                    self.shift = 0;
                    self.shift_count = 0;
                    self.control |= 0x0C;
                    return;
                }
                self.shift |= (val & 1) << self.shift_count;
                self.shift_count += 1;
                if self.shift_count == 5 {
                    let v = self.shift & 0x1F;
                    match (addr >> 13) & 3 {
                        0 => self.control = v,
                        1 => self.chr_bank0 = v,
                        2 => self.chr_bank1 = v,
                        _ => self.prg_bank = v,
                    }
                    self.shift = 0;
                    self.shift_count = 0;
                }
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
        match self.control & 3 {
            0 => Mirroring::SingleScreenLo,
            1 => Mirroring::SingleScreenHi,
            2 => Mirroring::Vertical,
            _ => Mirroring::Horizontal,
        }
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        if self.wram_disabled() {
            return None; // open bus
        }
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

    fn mmc1() -> Mmc1 {
        // 8 PRG banks (128KB), 4 CHR banks (16KB); each byte = its bank
        // index (16KB PRG banks, 4KB CHR banks).
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..4 * 0x1000).map(|i| (i / 0x1000) as u8).collect();
        Mmc1::new(prg, chr)
    }

    /// Write one value through the 5-bit serial interface (LSB first).
    fn serial_write(m: &mut Mmc1, addr: u16, val: u8) {
        for i in 0..5 {
            m.cpu_write(addr, (val >> i) & 1);
        }
    }

    #[test]
    fn power_on_fixes_last_prg_bank() {
        let mut m = mmc1();
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xC000), 7);
        assert_eq!(m.cpu_read(0xFFFC), 7); // reset vector reachable
    }

    #[test]
    fn serial_write_selects_prg_bank() {
        let mut m = mmc1();
        serial_write(&mut m, 0xE000, 3);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xC000), 7); // still fixed last
    }

    #[test]
    fn reset_bit_clears_shift_and_forces_mode3() {
        let mut m = mmc1();
        // Switch to 32KB mode first.
        serial_write(&mut m, 0x8000, 0x00);
        // Two stray bits, then a reset write.
        m.cpu_write(0xE000, 1);
        m.cpu_write(0xE000, 1);
        m.cpu_write(0xE000, 0x80);
        // Mode 3 restored: last bank fixed at $C000.
        assert_eq!(m.cpu_read(0xC000), 7);
        // Shift state cleared: a fresh 5-write sequence works.
        serial_write(&mut m, 0xE000, 2);
        assert_eq!(m.cpu_read(0x8000), 2);
    }

    #[test]
    fn prg_modes() {
        let mut m = mmc1();
        serial_write(&mut m, 0xE000, 5);
        // 32KB mode: low bit of prg_bank ignored.
        serial_write(&mut m, 0x8000, 0x00);
        assert_eq!(m.cpu_read(0x8000), 4);
        assert_eq!(m.cpu_read(0xC000), 5);
        // Fix-first mode: bank 0 at $8000, switchable at $C000.
        serial_write(&mut m, 0x8000, 0x08);
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xC000), 5);
    }

    #[test]
    fn chr_4k_vs_8k() {
        let mut m = mmc1();
        // 8KB mode (default): chr_bank0 low bit ignored.
        serial_write(&mut m, 0xA000, 3);
        assert_eq!(m.ppu_read(0x0000), 2);
        assert_eq!(m.ppu_read(0x1000), 3);
        // 4KB mode: independent banks.
        serial_write(&mut m, 0x8000, 0x1C);
        serial_write(&mut m, 0xA000, 1);
        serial_write(&mut m, 0xC000, 3);
        assert_eq!(m.ppu_read(0x0000), 1);
        assert_eq!(m.ppu_read(0x1000), 3);
    }

    #[test]
    fn mirroring_modes() {
        let mut m = mmc1();
        for (v, want) in [
            (0x0C, Mirroring::SingleScreenLo),
            (0x0D, Mirroring::SingleScreenHi),
            (0x0E, Mirroring::Vertical),
            (0x0F, Mirroring::Horizontal),
        ] {
            serial_write(&mut m, 0x8000, v);
            assert_eq!(m.mirroring(), want);
        }
    }

    #[test]
    fn prg_ram_read_write() {
        let mut m = mmc1();
        m.cpu_write(0x6123, 0xCD);
        assert_eq!(m.prg_ram_read(0x6123), Some(0xCD));
    }

    #[test]
    fn wram_disable_bits() {
        // SNROM-style board: 8KB CHR RAM, 128KB PRG.
        let mut m = Mmc1::new(vec![0; 8 * 0x4000], vec![]);
        m.cpu_write(0x6000, 0xAA);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAA));
        // PRG bank register bit 4 disables WRAM on every board.
        serial_write(&mut m, 0xE000, 0x10);
        assert_eq!(m.prg_ram_read(0x6000), None);
        m.cpu_write(0x6000, 0xBB); // ignored while disabled
        serial_write(&mut m, 0xE000, 0x00);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAA));
        // SNROM only: CHR bank register bit 4 disables WRAM too.
        serial_write(&mut m, 0xA000, 0x10);
        assert_eq!(m.prg_ram_read(0x6000), None);
        serial_write(&mut m, 0xA000, 0x00);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAA));
    }

    #[test]
    fn chr_rom_board_bit4_keeps_wram_enabled() {
        // CHR ROM board: $A000 bit 4 is a CHR bank bit, not WRAM disable.
        let mut m = mmc1();
        m.cpu_write(0x6000, 0xAA);
        serial_write(&mut m, 0xA000, 0x10);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAA));
    }

    #[test]
    fn surom_512k_half_select() {
        // 32 PRG banks (512KB); bank index mod 256 per byte won't fit, so
        // store (bank index) in each byte truncated — use bank index
        // directly since 32 < 256.
        let prg: Vec<u8> = (0..32 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let mut m = Mmc1::new(prg, vec![]);
        // Fixed last bank is last of the LOW half until bit 4 set.
        assert_eq!(m.cpu_read(0xC000), 15);
        // Select high 256KB half via CHR bank 0 bit 4.
        serial_write(&mut m, 0xA000, 0x10);
        assert_eq!(m.cpu_read(0xC000), 31);
        serial_write(&mut m, 0xE000, 2);
        assert_eq!(m.cpu_read(0x8000), 18);
    }
}
