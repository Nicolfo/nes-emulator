use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Mapper 28 - **Action 53** (homebrew multicart, Damian Yerrick).
///
/// PRG up to 2MB, organised as 32KB "outer" banks each containing 16KB
/// "inner" banks. CHR is always CHR RAM (an 8KB window, up to 32KB total
/// addressed as 4 x 8KB pages). Mirroring is mapper-controlled.
///
/// The board uses a two-step register interface:
/// * a write to $5000-$5FFF selects which of four registers is targeted
///   (`reg_select = val & 0x81`; the target index is bits 0-1 of val),
/// * a write to $8000-$FFFF stores into the selected register.
///
/// IMPORTANT: the PRG/mode bank decode here is an APPROXIMATION. The
/// holy-mapperel M28 test ROM is the source of truth and the exact bank math
/// is expected to be tuned by the caller. All of the PRG mapping is isolated
/// in [`Action53::prg_offset`] so it is trivial to adjust.
#[derive(Clone, Serialize, Deserialize)]
pub struct Action53 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,

    /// Last value written to $5000-$5FFF, masked to `val & 0x81`. The board
    /// only decodes bit 7 and bit 0 of the select, giving four register
    /// addresses - $00, $01, $80, $81 - collapsed by [`Action53::reg_index`]
    /// to 0..3 (CHR, PRG inner, mode, PRG outer).
    reg_select: u8,

    /// reg $00 - CHR bank (bits 1-0 = CHR-RAM A14-A13, an 8KB page). Bit 4 is
    /// the 1-screen "M" page-select bit (see [`Action53::one_screen_hi`]).
    chr_bank: u8,
    /// reg $01 - PRG inner bank (low 4 bits). Bit 4 is also an M page bit.
    prg_inner: u8,
    /// reg $80 - mode: bits 1-0 mirroring, bits 3-2 PRG mode, bits 5-4 outer
    /// bank size (see [`Action53::prg_offset`]).
    mode: u8,
    /// reg $81 - PRG outer bank (32KB-granular outer bank index).
    prg_outer: u8,
    /// Latched 1-screen page select (CIRAM A10). Fed by mode-register bit 0 on
    /// a mode write and by the "M" bit (bit 4) of reg $00/$01 on those writes;
    /// the most recent write wins, matching the hardware's shared latch. Only
    /// consulted when the mode register selects a 1-screen mirroring mode.
    one_screen_hi: bool,
}

impl Action53 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        // Action 53 is CHR RAM in practice; if the header declared CHR ROM we
        // keep it and treat it as read-only, otherwise allocate CHR RAM. Boards
        // carry up to 32KB (4 x 8KB pages), so allocate the full window to allow
        // the reg-0 page select to address all four banks.
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x8000] } else { chr };
        Action53 {
            prg,
            chr,
            chr_is_ram,
            mirroring,
            reg_select: 0,
            chr_bank: 0,
            prg_inner: 0,
            mode: 0,
            // Power-on: the board maps the ROM's last 16KB into $C000-$FFFF so
            // the reset vector and mapper-detect routine are reachable. With
            // mode 0 (32K) an all-ones outer bank does exactly that - $C000
            // resolves to (0xFF<<1)|1, which wraps to the last bank.
            prg_outer: 0xFF,
            one_screen_hi: false,
        }
    }

    /// Map a CPU address in $8000-$FFFF to a 16KB PRG bank offset, per the
    /// Action 53 banking logic table. The 16KB bank number is built from the
    /// outer bank register ($81, 32KB-granular: contributes `outer << 1`) and
    /// the inner bank register ($01); the outer-bank-size field (`mode` bits
    /// 5-4) decides how many low bits the inner register replaces.
    ///
    /// * PRG mode 0/1 (`mode` bits 3-2): 32KB mode - both halves switchable,
    ///   with CPU A14 as the low bit and `size` inner bits above it.
    /// * PRG mode 2: $8000 fixed (outer forced to full 32KB, low bit 0),
    ///   $C000 switchable.
    /// * PRG mode 3: $C000 fixed (low bit 1), $8000 switchable.
    fn prg_offset(&self, addr: u16) -> usize {
        let banks = (self.prg.len() / 0x4000).max(1);
        let outer = self.prg_outer as usize;
        let inner = self.prg_inner as usize;
        let high = (addr >> 14) & 1 != 0; // false: $8000-$BFFF, true: $C000-$FFFF

        // Low (size+1) bits of the bank come from the switchable source; the
        // outer register supplies the bits above.
        let size = (self.mode >> 4) & 3;
        let mask = (1usize << (size + 1)) - 1;
        let base = (outer << 1) & !mask;
        // Switchable region: inner bits fill the low window.
        let switch = base | (inner & mask);

        let bank = match (self.mode >> 2) & 3 {
            // 32KB mode: A14 is the low bit, inner bits sit above it.
            0 | 1 => base | (((inner << 1) | usize::from(high)) & mask),
            // Fixed $8000 (forced full-width outer), switchable $C000.
            2 => {
                if high {
                    switch
                } else {
                    outer << 1
                }
            }
            // Fixed $C000 (forced full-width outer), switchable $8000.
            _ => {
                if high {
                    (outer << 1) | 1
                } else {
                    switch
                }
            }
        };

        (bank % banks) * 0x4000 + (addr as usize & 0x3FFF)
    }

    /// Map a PPU address in $0000-$1FFF to a byte offset in `chr`.
    fn chr_offset(&self, addr: u16) -> usize {
        let banks = (self.chr.len() / 0x2000).max(1);
        (self.chr_bank as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)
    }

    /// Collapse the masked select ($00/$01/$80/$81) to a register index 0..3:
    /// bit 0 is the low index bit, bit 7 the high one.
    fn reg_index(&self) -> u8 {
        (self.reg_select & 1) | ((self.reg_select >> 6) & 2)
    }

    /// Recompute `mirroring` from the mode register and the 1-screen latch.
    fn update_mirroring(&mut self) {
        self.mirroring = match self.mode & 0x03 {
            2 => Mirroring::Vertical,
            3 => Mirroring::Horizontal,
            // 1-screen: the page comes from the shared M latch.
            _ => {
                if self.one_screen_hi {
                    Mirroring::SingleScreenHi
                } else {
                    Mirroring::SingleScreenLo
                }
            }
        };
    }
}

impl Mapper for Action53 {
    crate::impl_mapper_savestate!(chr_is_ram = chr_is_ram);

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            // Register-select port: latch which register the next data write
            // targets. Only bits 7 and 0 are decoded.
            0x5000..=0x5FFF => self.reg_select = val & 0x81,
            // Data port: store into the selected register.
            0x8000..=0xFFFF => match self.reg_index() {
                0 => {
                    self.chr_bank = val & 0x03;
                    // Bit 4 is the M page bit; it overrides the 1-screen latch.
                    self.one_screen_hi = val & 0x10 != 0;
                    self.update_mirroring();
                }
                1 => {
                    self.prg_inner = val & 0x0F;
                    self.one_screen_hi = val & 0x10 != 0;
                    self.update_mirroring();
                }
                2 => {
                    self.mode = val;
                    // Mode bit 0 also drives the 1-screen latch.
                    self.one_screen_hi = val & 0x01 != 0;
                    self.update_mirroring();
                }
                _ => self.prg_outer = val,
            },
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2MB PRG (64 x 32KB outer banks), each 16KB tagged with its 16KB bank
    /// index so a read reveals the bank that was selected.
    fn mapper() -> Action53 {
        let prg: Vec<u8> = (0..128 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        Action53::new(prg, vec![], Mirroring::Vertical)
    }

    /// Helper: select a register (logical index 0..3) then write its data byte.
    fn write_reg(m: &mut Action53, reg: u8, val: u8) {
        let sel = match reg {
            0 => 0x00,
            1 => 0x01,
            2 => 0x80,
            _ => 0x81,
        };
        m.cpu_write(0x5000, sel);
        m.cpu_write(0x8000, val);
    }

    #[test]
    fn register_select_round_trip() {
        let mut m = mapper();
        // Only bits 7 and 0 of the select survive; they form the register index.
        m.cpu_write(0x5000, 0xFF);
        assert_eq!(m.reg_select, 0x81);
        assert_eq!(m.reg_index(), 3);
        m.cpu_write(0x5000, 0x80);
        assert_eq!(m.reg_index(), 2);

        write_reg(&mut m, 0, 0xFF); // CHR bank masks to 2 bits
        assert_eq!(m.chr_bank, 0x03);
        write_reg(&mut m, 1, 0xFF); // inner masks to 4 bits
        assert_eq!(m.prg_inner, 0x0F);
        write_reg(&mut m, 3, 0x2A); // outer stored raw
        assert_eq!(m.prg_outer, 0x2A);
    }

    #[test]
    fn prg_power_on_maps_last_bank_at_c000() {
        // Power-on: mode 0 (32KB), outer = 0xFF. With 128 x 16KB banks the high
        // half resolves to bank 127 (the ROM's last 16KB) so the reset vector
        // and mapper-detect routine are reachable; the low half is 126.
        let mut m = mapper();
        assert_eq!(m.cpu_read(0xC000), 127);
        assert_eq!(m.cpu_read(0x8000), 126);
    }

    #[test]
    fn prg_32k_mode_bank_select() {
        let mut m = mapper();
        // PRG mode 0 (mode bits 3-2 = 00), size 0: 32KB BNROM-style switch.
        write_reg(&mut m, 2, 0x00);
        write_reg(&mut m, 3, 3); // outer 3 -> 16KB banks 6 and 7
        assert_eq!(m.cpu_read(0x8000), 6); // low half
        assert_eq!(m.cpu_read(0xC000), 7); // high half
        // inner is ignored in 32KB mode with size 0.
        write_reg(&mut m, 1, 0x0F);
        assert_eq!(m.cpu_read(0x8000), 6);
    }

    #[test]
    fn prg_fixed_c000_mode_switches_8000() {
        let mut m = mapper();
        // PRG mode 3 (mode bits 3-2 = 11 -> 0x0C), size 0: switch $8000, fix the
        // odd high bank at $C000 (the UNROM-like layout the menu uses).
        write_reg(&mut m, 2, 0x0C);
        write_reg(&mut m, 3, 5); // outer 5 -> base 10, fixed high = 11
        write_reg(&mut m, 1, 0);
        assert_eq!(m.cpu_read(0x8000), 10);
        assert_eq!(m.cpu_read(0xC000), 11);
        write_reg(&mut m, 1, 1);
        assert_eq!(m.cpu_read(0x8000), 11);
        assert_eq!(m.cpu_read(0xC000), 11); // stays fixed
    }

    #[test]
    fn prg_size_field_widens_inner_window() {
        let mut m = mapper();
        // PRG mode 3, size 1 (mode 0x1C): inner is a 2-bit window, outer above.
        write_reg(&mut m, 2, 0x1C);
        write_reg(&mut m, 3, 2); // outer 2 -> base (2<<1)&~3 = 4
        write_reg(&mut m, 1, 3); // inner 3 (masked to 2 bits) -> 4|3 = 7
        assert_eq!(m.cpu_read(0x8000), 7);
        write_reg(&mut m, 1, 1); // inner 1 -> 4|1 = 5
        assert_eq!(m.cpu_read(0x8000), 5);
    }

    #[test]
    fn mirroring_mode_decode() {
        let mut m = mapper();
        write_reg(&mut m, 2, 0); // bits 1-0 = 00 -> 1-screen, latch low
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        write_reg(&mut m, 2, 1); // bit 0 drives the latch high
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
        write_reg(&mut m, 2, 2);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        write_reg(&mut m, 2, 3);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn one_screen_m_bit_overrides() {
        let mut m = mapper();
        write_reg(&mut m, 2, 0); // 1-screen, latch low from mode bit 0
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        // The M bit (bit 4) of the CHR register flips the page.
        write_reg(&mut m, 0, 0x10);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
        // ...and of the PRG inner register too.
        write_reg(&mut m, 1, 0x00);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
    }

    #[test]
    fn chr_ram_rw_and_bank_switch() {
        // Empty CHR -> 32KB CHR RAM -> 4 x 8KB pages.
        let prg: Vec<u8> = vec![0; 0x8000];
        let mut m = Action53::new(prg, vec![], Mirroring::Vertical);
        assert!(m.chr_is_ram);
        // page 0
        write_reg(&mut m, 0, 0);
        m.ppu_write(0x0010, 0xAA);
        assert_eq!(m.ppu_read(0x0010), 0xAA);
        // switch to page 1: same address is a different byte.
        write_reg(&mut m, 0, 1);
        assert_eq!(m.ppu_read(0x0010), 0x00);
        m.ppu_write(0x0010, 0xBB);
        assert_eq!(m.ppu_read(0x0010), 0xBB);
        // back to page 0 keeps its value.
        write_reg(&mut m, 0, 0);
        assert_eq!(m.ppu_read(0x0010), 0xAA);
    }
}
