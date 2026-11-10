//! Nintendo Vs. System (Vs. UniSystem) arcade hardware.
//!
//! A Vs. cabinet is an NES board with three differences that matter to
//! software:
//!
//! - **RGB PPUs.** The RP2C03/RC2C03, RP2C04 and RC2C05 output RGB directly
//!   from a 64-entry colour ROM instead of composite video. The 2C03 ROM
//!   approximates the NES palette; each of the four 2C04 variants stores the
//!   same colours in a different, scrambled order (a cheap copy protection:
//!   a game only looks right on the PPU it shipped with). On RGB PPUs the
//!   emphasis bits of $2001 force a channel to full brightness. The 2C05
//!   variants swap $2000/$2001 and return an identification value in the low
//!   bits of $2002.
//! - **Coin-op I/O.** $4016 reads add the service button, DIP switches 1-2
//!   and both coin slots; $4017 reads carry DIP switches 3-8.
//! - **Mapper 99** (the "Vs. UniSystem" discrete board) switches CHR (and
//!   Vs. Gumshoe's extra PRG) with bit 2 of $4016 writes.
//!
//! The header says which PPU a dump needs: NES 2.0 byte 13 (low nibble).
//! Plain iNES Vs. dumps can't, so they default to the RP2C03 - the right
//! choice for most Vs. titles, though 2C04 games then show wrong colours.
//!
//! Not emulated: the Vs. DualSystem (two linked boards: Vs. Tennis, Vs.
//! Baseball, Raid on Bungeling Bay...), and the protection chips of
//! RBI Baseball, TKO Boxing and Super Xevious.

use serde::{Deserialize, Serialize};

/// Which RGB PPU a Vs. board carries (NES 2.0 byte 13 low nibble).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VsPpu {
    /// RP2C03B/G, RC2C03C: the standard RGB palette.
    Rp2c03,
    /// RC2C03B: 2C03 with a few slightly different colours.
    Rc2c03b,
    /// RP2C04-0001 .. -0004 (1-4): scrambled palettes.
    Rp2c04(u8),
    /// RC2C05-01 .. -05 (1-5): 2C03 palette, $2000/$2001 swapped and an ID
    /// in $2002.
    Rc2c05(u8),
}

impl VsPpu {
    /// Decode NES 2.0 byte 13's low nibble.
    pub fn from_nes2(nibble: u8) -> Self {
        match nibble & 0x0F {
            2..=5 => VsPpu::Rp2c04(nibble - 1),
            6 => VsPpu::Rc2c03b,
            8..=0xC => VsPpu::Rc2c05(nibble - 7),
            _ => VsPpu::Rp2c03,
        }
    }

    /// 2C05: $2000 and $2001 trade places.
    pub fn swaps_ctrl_mask(self) -> bool {
        matches!(self, VsPpu::Rc2c05(_))
    }

    /// 2C05: the value the chip drives on the low bits of $2002, if any.
    pub fn status_id(self) -> Option<u8> {
        match self {
            VsPpu::Rc2c05(1) | VsPpu::Rc2c05(4) => Some(0x1B),
            VsPpu::Rc2c05(2) => Some(0x3D),
            VsPpu::Rc2c05(3) => Some(0x1C),
            _ => None,
        }
    }

    /// RGB for palette index `index` (0-63) with $2001 `mask`'s emphasis
    /// bits applied the RGB-PPU way: each set bit forces its channel
    /// (bit 5 red, 6 green, 7 blue) to full.
    pub fn rgb(self, index: u8, mask: u8) -> [u8; 3] {
        let i = (index & 0x3F) as usize;
        let c = match self {
            VsPpu::Rp2c03 | VsPpu::Rc2c05(_) => PALETTE_2C03[i],
            VsPpu::Rc2c03b => PALETTE_RC2C03B[i],
            VsPpu::Rp2c04(n) => PALETTE_2C04[LUT_2C04[(n.clamp(1, 4) - 1) as usize][i] as usize],
        };
        let mut rgb = [(c >> 6) & 7, (c >> 3) & 7, c & 7].map(|v| v as u8);
        for (ch, bit) in [0x20u8, 0x40, 0x80].iter().enumerate() {
            if mask & bit != 0 {
                rgb[ch] = 7;
            }
        }
        rgb.map(|v| (v as u16 * 255 / 7) as u8)
    }
}

/// Coin-op inputs the host drives, read back through $4016/$4017.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VsIo {
    pub coin1: bool,
    pub coin2: bool,
    pub service: bool,
    /// DIP switches 1-8 (bit 0 = switch 1).
    pub dip: u8,
}

impl VsIo {
    /// Extra bits of a $4016 read (bit 0, the controller, is ORed in by
    /// the bus): service (2), DIP 1-2 (3-4), coin 1 (5), coin 2 (6).
    pub fn port1_bits(&self) -> u8 {
        (self.service as u8) << 2
            | (self.dip & 0x03) << 3
            | (self.coin1 as u8) << 5
            | (self.coin2 as u8) << 6
    }

    /// Extra bits of a $4017 read: DIP switches 3-8 in bits 2-7.
    pub fn port2_bits(&self) -> u8 {
        self.dip & 0xFC
    }
}

/// Vs. hardware described by a ROM header, if it is a Vs. dump.
pub fn detect(header: &[u8]) -> Option<VsPpu> {
    if header.len() < 16 || &header[0..4] != b"NES\x1A" {
        return None;
    }
    let nes2 = header[7] & 0x0C == 0x08;
    if nes2 {
        // NES 2.0 console type 1 = Vs. System; byte 13 names the PPU.
        (header[7] & 0x03 == 1).then(|| VsPpu::from_nes2(header[13]))
    } else {
        // Plain iNES: flags7 bit 0. Old dumps with junk in bytes 12-15
        // ("DiskDude!") can't be trusted to mean it.
        let clean = header[12..16].iter().all(|&b| b == 0);
        (clean && header[7] & 0x01 != 0).then_some(VsPpu::Rp2c03)
    }
}

// Palette data from Nintendulator (Quietust), octal RGB with 3 bits per
// channel.

#[rustfmt::skip]
const PALETTE_2C03: [u16; 64] = [
    0o333,0o014,0o006,0o326,0o403,0o503,0o510,0o420,0o320,0o120,0o031,0o040,0o022,0o000,0o000,0o000,
    0o555,0o036,0o027,0o407,0o507,0o704,0o700,0o630,0o430,0o140,0o040,0o053,0o044,0o000,0o000,0o000,
    0o777,0o357,0o447,0o637,0o707,0o737,0o740,0o750,0o660,0o360,0o070,0o276,0o077,0o000,0o000,0o000,
    0o777,0o567,0o657,0o757,0o747,0o755,0o764,0o772,0o773,0o572,0o473,0o276,0o467,0o000,0o000,0o000,
];

#[rustfmt::skip]
const PALETTE_RC2C03B: [u16; 64] = [
    0o333,0o014,0o006,0o326,0o403,0o503,0o510,0o420,0o320,0o100,0o031,0o040,0o022,0o000,0o000,0o000,
    0o555,0o016,0o027,0o407,0o507,0o704,0o700,0o630,0o430,0o140,0o040,0o053,0o044,0o000,0o000,0o000,
    0o777,0o357,0o447,0o637,0o707,0o717,0o740,0o750,0o660,0o340,0o070,0o276,0o077,0o000,0o000,0o000,
    0o777,0o547,0o657,0o757,0o747,0o755,0o764,0o772,0o773,0o552,0o473,0o276,0o467,0o000,0o000,0o000,
];

/// The 2C04 colour ROM in its canonical (unscrambled) order.
#[rustfmt::skip]
const PALETTE_2C04: [u16; 64] = [
    0o333,0o014,0o006,0o326,0o403,0o503,0o510,0o420,0o320,0o120,0o031,0o040,0o022,0o111,0o003,0o020,
    0o555,0o036,0o027,0o407,0o507,0o704,0o700,0o630,0o430,0o140,0o040,0o053,0o044,0o222,0o200,0o310,
    0o777,0o357,0o447,0o637,0o707,0o737,0o740,0o750,0o660,0o360,0o070,0o276,0o077,0o444,0o000,0o772,
    0o777,0o567,0o657,0o757,0o747,0o755,0o764,0o770,0o773,0o572,0o473,0o276,0o467,0o666,0o653,0o760,
];

/// Per-variant scrambling: palette index -> canonical 2C04 colour.
#[rustfmt::skip]
const LUT_2C04: [[u8; 64]; 4] = [
    [
        0x35,0x23,0x16,0x22,0x1C,0x09,0x1D,0x15,0x20,0x00,0x27,0x05,0x04,0x28,0x08,0x20,
        0x21,0x3E,0x1F,0x29,0x3C,0x32,0x36,0x12,0x3F,0x2B,0x2E,0x1E,0x3D,0x2D,0x24,0x01,
        0x0E,0x31,0x33,0x2A,0x2C,0x0C,0x1B,0x14,0x2E,0x07,0x34,0x06,0x13,0x02,0x26,0x2E,
        0x2E,0x19,0x10,0x0A,0x39,0x03,0x37,0x17,0x0F,0x11,0x0B,0x0D,0x38,0x25,0x18,0x3A,
    ],
    [
        0x2E,0x27,0x18,0x39,0x3A,0x25,0x1C,0x31,0x16,0x13,0x38,0x34,0x20,0x23,0x3C,0x0B,
        0x0F,0x21,0x06,0x3D,0x1B,0x29,0x1E,0x22,0x1D,0x24,0x0E,0x2B,0x32,0x08,0x2E,0x03,
        0x04,0x36,0x26,0x33,0x11,0x1F,0x10,0x02,0x14,0x3F,0x00,0x09,0x12,0x2E,0x28,0x20,
        0x3E,0x0D,0x2A,0x17,0x0C,0x01,0x15,0x19,0x2E,0x2C,0x07,0x37,0x35,0x05,0x0A,0x2D,
    ],
    [
        0x14,0x25,0x3A,0x10,0x0B,0x20,0x31,0x09,0x01,0x2E,0x36,0x08,0x15,0x3D,0x3E,0x3C,
        0x22,0x1C,0x05,0x12,0x19,0x18,0x17,0x1B,0x00,0x03,0x2E,0x02,0x16,0x06,0x34,0x35,
        0x23,0x0F,0x0E,0x37,0x0D,0x27,0x26,0x20,0x29,0x04,0x21,0x24,0x11,0x2D,0x2E,0x1F,
        0x2C,0x1E,0x39,0x33,0x07,0x2A,0x28,0x1D,0x0A,0x2E,0x32,0x38,0x13,0x2B,0x3F,0x0C,
    ],
    [
        0x18,0x03,0x1C,0x28,0x2E,0x35,0x01,0x17,0x10,0x1F,0x2A,0x0E,0x36,0x37,0x0B,0x39,
        0x25,0x1E,0x12,0x34,0x2E,0x1D,0x06,0x26,0x3E,0x1B,0x22,0x19,0x04,0x2E,0x3A,0x21,
        0x05,0x0A,0x07,0x02,0x13,0x14,0x00,0x15,0x0C,0x3D,0x11,0x0F,0x0D,0x38,0x2D,0x24,
        0x33,0x20,0x08,0x16,0x3F,0x2B,0x20,0x3C,0x2E,0x27,0x23,0x31,0x29,0x32,0x2C,0x09,
    ],
];

#[cfg(test)]
mod tests {
    use super::*;

    fn header(flags7: u8, byte13: u8) -> Vec<u8> {
        let mut h = vec![0u8; 16];
        h[0..4].copy_from_slice(b"NES\x1A");
        h[4] = 2;
        h[5] = 1;
        h[7] = flags7;
        h[13] = byte13;
        h
    }

    #[test]
    fn detection_from_ines_and_nes2() {
        assert_eq!(detect(&header(0x00, 0)), None);
        assert_eq!(detect(&header(0x01, 0)), Some(VsPpu::Rp2c03));
        // Junk in bytes 12-15 voids the iNES Vs. bit.
        let mut junk = header(0x01, 0);
        junk[12..16].copy_from_slice(b"Dude");
        assert_eq!(detect(&junk), None);
        // NES 2.0 console type 1 with PPU types from byte 13.
        assert_eq!(detect(&header(0x09, 0x03)), Some(VsPpu::Rp2c04(2)));
        assert_eq!(detect(&header(0x09, 0x06)), Some(VsPpu::Rc2c03b));
        assert_eq!(detect(&header(0x09, 0x09)), Some(VsPpu::Rc2c05(2)));
        assert_eq!(detect(&header(0x09, 0x01)), Some(VsPpu::Rp2c03));
        // NES 2.0 but a home console: not Vs. even with bit 0's twin unset.
        assert_eq!(detect(&header(0x08, 0x03)), None);
    }

    #[test]
    fn rp2c04_scrambles_the_colour_rom() {
        // Nintendulator's pre-permuted RP2C04-0001 ROM starts 755,637,700;
        // the LUT path must reproduce it.
        let full = |c: u16| [(c >> 6) & 7, (c >> 3) & 7, c & 7].map(|v| (v * 255 / 7) as u8);
        let p = VsPpu::Rp2c04(1);
        assert_eq!(p.rgb(0x00, 0), full(0o755));
        assert_eq!(p.rgb(0x01, 0), full(0o637));
        assert_eq!(p.rgb(0x02, 0), full(0o700));
        // RP2C04-0004 index $3F = canonical $09 = 0o120 (Nintendulator: 0120).
        assert_eq!(VsPpu::Rp2c04(4).rgb(0x3F, 0), full(0o120));
        // Every variant uses each canonical colour; none index out of range.
        for lut in &LUT_2C04 {
            assert!(lut.iter().all(|&i| i < 64));
        }
    }

    #[test]
    fn emphasis_forces_full_channels() {
        assert_eq!(VsPpu::Rp2c03.rgb(0x0F, 0), [0, 0, 0]);
        assert_eq!(VsPpu::Rp2c03.rgb(0x20, 0), [255, 255, 255]);
        assert_eq!(VsPpu::Rp2c03.rgb(0x0F, 0x20), [255, 0, 0]);
        assert_eq!(VsPpu::Rp2c03.rgb(0x0F, 0xC0), [0, 255, 255]);
    }

    #[test]
    fn rc2c05_ids_and_swap() {
        assert!(VsPpu::Rc2c05(1).swaps_ctrl_mask());
        assert!(!VsPpu::Rp2c03.swaps_ctrl_mask());
        assert_eq!(VsPpu::Rc2c05(1).status_id(), Some(0x1B));
        assert_eq!(VsPpu::Rc2c05(2).status_id(), Some(0x3D));
        assert_eq!(VsPpu::Rc2c05(3).status_id(), Some(0x1C));
        assert_eq!(VsPpu::Rc2c05(5).status_id(), None);
    }

    #[test]
    fn coin_dip_service_bits() {
        let io = VsIo {
            coin1: true,
            coin2: false,
            service: true,
            dip: 0b1010_0110,
        };
        assert_eq!(io.port1_bits(), 0x04 | (0b10 << 3) | 0x20);
        assert_eq!(io.port2_bits(), 0b1010_0100);
    }
}
