use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Namco 175 and Namco 340 (iNES mapper 210). These are cost-reduced Namco
/// 163 variants that keep the N163 PRG/CHR banking register map but drop the
/// expansion audio, the IRQ counter, and the CHR-in-nametable trick.
///
/// Register map (shared with N163):
/// - CHR: eight 1KB banks selected by $8000/$8800/.../$B800 ($800 step). The
///   write value is a raw 1KB CHR page ($00-$FF).
/// - PRG: three 8KB switchable banks. $E000 = $8000 bank (low 6 bits),
///   $E800 = $A000 bank (low 6 bits), $F000 = $C000 bank (low 6 bits). The
///   last 8KB ($E000-$FFFF of the CPU map) is fixed to the final PRG bank.
///
/// The NES 2.0 submapper picks the board:
/// - Namco 340 (submapper 2): mapper-controlled mirroring via $E000 bits 7-6
///   (0 = 1-screen A, 1 = vertical, 2 = 1-screen B, 3 = horizontal). No PRG
///   RAM.
/// - Namco 175 (submapper 1): hardwired mirroring from the iNES header, plus
///   an 8KB battery PRG RAM at $6000-$7FFF gated by an enable bit at
///   $C000 bit 0.
///
/// When the submapper is unspecified (submapper 0 / plain iNES) we honor the
/// header mirroring until the game writes the $E000 mirroring bits, at which
/// point we commit to the 340 register-driven path. A submapper-locked board
/// keeps its fixed interpretation. FourScreen headers are never overridden.
#[derive(Serialize, Deserialize)]
pub struct Namco175340 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    #[serde(default)]
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    prg_banks: [u8; 3],
    chr_banks: [u8; 8],
    /// Mirroring taken from the iNES header (the 175 hardwired case, and the
    /// default until a 340 mirroring write is seen).
    header_mirroring: Mirroring,
    /// Mirroring last selected via the $E000 bits 7-6 (340 path).
    reg_mirroring: Mirroring,
    /// Set once a game writes the $E000 mirroring bits (when not locked);
    /// switches us onto the 340 register-driven mirroring path.
    use_reg_mirroring: bool,
    /// When the NES 2.0 submapper pins the board down (175 or 340), the mode is
    /// locked and $E000 mirroring writes can no longer flip it.
    mode_locked: bool,
    /// 175 PRG RAM enable ($C000 bit 0). Defaults enabled so a 175 game that
    /// never bothers writing the register still sees working save RAM, and so
    /// the field is harmless for 340 (which has no PRG RAM accesses anyway).
    prg_ram_enabled: bool,
}

impl Namco175340 {
    /// `submapper` is the NES 2.0 submapper: 1 = Namco 175 (hardwired header
    /// mirroring + battery PRG RAM), 2 = Namco 340 (mapper-controlled
    /// mirroring), 0 = unspecified (plain iNES) - fall back to a heuristic that
    /// honors the header until the game writes the $E000 mirroring bits.
    pub fn new(submapper: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let (use_reg_mirroring, mode_locked) = match submapper {
            1 => (false, true),  // Namco 175: hardwired header mirroring
            2 => (true, true),   // Namco 340: register-driven mirroring
            _ => (false, false), // unspecified: heuristic, may flip on a write
        };
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Namco175340 {
            prg,
            chr,
            chr_is_ram,
            prg_ram: vec![0; 0x2000],
            prg_banks: [0; 3],
            chr_banks: [0; 8],
            header_mirroring: mirroring,
            // Seed the register path with the header value so a 340 game that
            // reads mirroring before its first $E000 write still behaves.
            reg_mirroring: mirroring,
            use_reg_mirroring,
            mode_locked,
            prg_ram_enabled: true,
        }
    }

    fn chr_offset(&self, bank: u8, addr: u16) -> usize {
        let banks = self.chr.len() / 0x400;
        (bank as usize % banks) * 0x400 + (addr as usize & 0x3FF)
    }

    fn chr_byte(&self, bank: u8, addr: u16) -> u8 {
        self.chr[self.chr_offset(bank, addr)]
    }
}

impl Mapper for Namco175340 {
    crate::impl_mapper_savestate!(prg, chr, prg_ram);

    fn set_ram_sizes(&mut self, prg_ram: usize, chr_ram: usize) {
        if prg_ram > 0 {
            self.prg_ram = vec![0; prg_ram];
        }
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x2000).max(1);
        let bank = match addr {
            0x8000..=0x9FFF => self.prg_banks[0] as usize % banks,
            0xA000..=0xBFFF => self.prg_banks[1] as usize % banks,
            0xC000..=0xDFFF => self.prg_banks[2] as usize % banks,
            // $E000-$FFFF is hardwired to the last 8KB bank.
            _ => banks - 1,
        };
        self.prg[bank * 0x2000 + (addr as usize & 0x1FFF)]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            // 8KB battery PRG RAM (Namco 175). Harmless on 340.
            0x6000..=0x7FFF if self.prg_ram_enabled => {
                self.prg_ram[(addr & 0x1FFF) as usize] = val;
            }
            // Eight 1KB CHR bank registers, $800 apart ($8000..=$BFFF).
            0x8000..=0xBFFF => {
                self.chr_banks[((addr - 0x8000) >> 11) as usize] = val;
            }
            // Namco 175 PRG RAM enable ($C000 bit 0). 340 has no PRG RAM and
            // never writes here, so this is inert for it.
            0xC000..=0xC7FF => {
                self.prg_ram_enabled = val & 0x01 != 0;
            }
            // PRG bank at $8000 (low 6 bits). Bits 7-6 select 340 mirroring.
            0xE000..=0xE7FF => {
                self.prg_banks[0] = val & 0x3F;
                // 340 mirroring path: any write here means the cart drives
                // mirroring, so latch it and switch onto the register path.
                self.reg_mirroring = match (val >> 6) & 0x03 {
                    0 => Mirroring::SingleScreenLo,
                    1 => Mirroring::Vertical,
                    2 => Mirroring::SingleScreenHi,
                    _ => Mirroring::Horizontal,
                };
                // In the unspecified case, the first $E000 mirroring write
                // commits us to the 340 path. A submapper-locked board keeps
                // its fixed interpretation. FourScreen headers are never
                // overridden.
                if !self.mode_locked && self.header_mirroring != Mirroring::FourScreen {
                    self.use_reg_mirroring = true;
                }
            }
            // PRG bank at $A000 (low 6 bits).
            0xE800..=0xEFFF => self.prg_banks[1] = val & 0x3F,
            // PRG bank at $C000 (low 6 bits).
            0xF000..=0xF7FF => self.prg_banks[2] = val & 0x3F,
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x2000 {
            let bank = self.chr_banks[(addr >> 10) as usize & 7];
            self.chr_byte(bank, addr)
        } else {
            0
        }
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        // Real 175/340 boards are CHR ROM; the RAM path only serves zero-CHR
        // images (iNES CHR RAM convention). Nametable writes are routed to
        // CIRAM by the PPU using mirroring() below.
        if self.chr_is_ram && addr < 0x2000 {
            let bank = self.chr_banks[(addr >> 10) as usize & 7];
            let off = self.chr_offset(bank, addr);
            self.chr[off] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        // FourScreen header boards are never overridden. Otherwise use the
        // 340 register-driven value once a game has written it, else fall
        // back to the hardwired header mirroring (the 175 case / default).
        if self.header_mirroring == Mirroring::FourScreen {
            Mirroring::FourScreen
        } else if self.use_reg_mirroring {
            self.reg_mirroring
        } else {
            self.header_mirroring
        }
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        if self.prg_ram_enabled {
            Some(self.prg_ram[(addr & 0x1FFF) as usize])
        } else {
            None
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

    fn mapper_sub(submapper: u8, mirroring: Mirroring) -> Namco175340 {
        // 8 x 8KB PRG, 64 x 1KB CHR; each byte equals its bank index.
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..64 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Namco175340::new(submapper, prg, chr, mirroring)
    }

    /// Default test board: submapper 0 (unspecified, heuristic).
    fn mapper(mirroring: Mirroring) -> Namco175340 {
        mapper_sub(0, mirroring)
    }

    #[test]
    fn chr_1kb_banking() {
        let mut m = mapper(Mirroring::Vertical);
        m.cpu_write(0x8000, 33); // PPU $0000-$03FF
        m.cpu_write(0x8800, 17); // PPU $0400-$07FF
        m.cpu_write(0xB800, 7); // PPU $1C00-$1FFF
        assert_eq!(m.ppu_read(0x0000), 33);
        assert_eq!(m.ppu_read(0x0400), 17);
        assert_eq!(m.ppu_read(0x1C00), 7);
        // CHR bank index wraps modulo the available banks.
        m.cpu_write(0x9000, 64); // 64 % 64 == 0
        assert_eq!(m.ppu_read(0x0800), 0);
    }

    #[test]
    fn prg_three_banks_and_fixed_last() {
        let mut m = mapper(Mirroring::Vertical);
        m.cpu_write(0xE000, 3);
        m.cpu_write(0xE800, 4);
        m.cpu_write(0xF000, 5);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xA000), 4);
        assert_eq!(m.cpu_read(0xC000), 5);
        // $E000-$FFFF is fixed to the last (8th) bank.
        assert_eq!(m.cpu_read(0xE000), 7);
        assert_eq!(m.cpu_read(0xFFFF), 7);
        // Only the low 6 bits select the bank ($E000 bits 7-6 are mirroring).
        m.cpu_write(0xE000, 0xC0 | 2); // mirroring bits set, bank = 2
        assert_eq!(m.cpu_read(0x8000), 2);
    }

    #[test]
    fn prg_ram_read_write() {
        let mut m = mapper(Mirroring::Vertical);
        // Enabled by default.
        m.cpu_write(0x6000, 0xAB);
        m.cpu_write(0x7FFF, 0xCD);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAB));
        assert_eq!(m.prg_ram_read(0x7FFF), Some(0xCD));
        // Disabling via $C000 bit 0 gates reads/writes (175 behaviour).
        m.cpu_write(0xC000, 0x00);
        assert_eq!(m.prg_ram_read(0x6000), None);
        m.cpu_write(0x6000, 0xFF); // ignored while disabled
        m.cpu_write(0xC000, 0x01); // re-enable
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAB));
        // Raw battery view bypasses the enable gate.
        assert_eq!(m.prg_ram().unwrap()[0], 0xAB);
    }

    #[test]
    fn mirroring_honors_header_until_register_write() {
        // 175-like default: report the header mirroring.
        let mut m = mapper(Mirroring::Horizontal);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        // 340 path: a write to $E000 mirroring bits takes over.
        m.cpu_write(0xE000, 0b0000_0000); // bits 7-6 = 0 -> 1-screen A
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        m.cpu_write(0xE000, 0b0100_0000); // 1 -> vertical
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0xE000, 0b1000_0000); // 2 -> 1-screen B
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
        m.cpu_write(0xE000, 0b1100_0000); // 3 -> horizontal
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn submapper_locks_mirroring_mode() {
        // Submapper 1 (Namco 175): hardwired header mirroring; $E000 writes to
        // the mirroring bits are ignored.
        let mut m175 = mapper_sub(1, Mirroring::Vertical);
        m175.cpu_write(0xE000, 0b1100_0000); // would select horizontal
        assert_eq!(m175.mirroring(), Mirroring::Vertical);
        // Submapper 2 (Namco 340): register-driven mirroring from the start.
        let mut m340 = mapper_sub(2, Mirroring::Vertical);
        m340.cpu_write(0xE000, 0b1100_0000); // horizontal
        assert_eq!(m340.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn four_screen_header_never_overridden() {
        let mut m = mapper(Mirroring::FourScreen);
        m.cpu_write(0xE000, 0b1100_0000); // attempt to select horizontal
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
    }
}
