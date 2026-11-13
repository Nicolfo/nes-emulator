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
//! Plain iNES dumps can't, so they are looked up in a small game database
//! (`vs_db`, by PRG+CHR CRC32); unknown ones default to the RP2C03 - the
//! right choice for most Vs. titles.
//!
//! Per-game extras, from NES 2.0 bytes 13 (high nibble) and 15 or the
//! database:
//!
//! - **Protection chips.** RBI Baseball and TKO Boxing read a sequence from
//!   $5E01 (reset by a $5E00 read); Super Xevious answers four magic
//!   addresses in $5400-$57FF; Ice Climber (Japan) and Raid on Bungeling Bay
//!   expect the Start line of each controller held high.
//! - **Controller wiring.** Many games read player 1 from $4017 ("swapped");
//!   Vs. Pinball (Japan) trades P1 B and P2 A. On every cabinet the start
//!   buttons drive the Select line, so the host's Start and Select are
//!   swapped to keep "Start starts the game".
//! - **Vs. Zapper.** Duck Hunt, Hogan's Alley and Freedom Force read the gun
//!   serially through $4016 D0: 8 bits latched on strobe, D4 always set, D6
//!   light seen, D7 trigger.
//! - **DualSystem.** Two complete boards that share 2KB of work RAM and can
//!   interrupt each other (Vs. Tennis, Vs. Baseball, Wrecking Crew...): see
//!   [`crate::nes::Nes`].

use serde::{Deserialize, Serialize};

use crate::controller::{BTN_A, BTN_B, BTN_SELECT, BTN_START};

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
        let n = nibble & 0x0F;
        match n {
            2..=5 => VsPpu::Rp2c04(n - 1),
            6 => VsPpu::Rc2c03b,
            8..=0xC => VsPpu::Rc2c05(n - 7),
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

    /// Decode a game-database PPU model (see [`crate::vs_db`]).
    fn from_db(model: u8) -> Self {
        match model {
            2..=5 => VsPpu::Rp2c04(model - 1),
            6..=10 => VsPpu::Rc2c05(model - 5),
            _ => VsPpu::Rp2c03,
        }
    }
}

/// Board variant: protection hardware, or a DualSystem (NES 2.0 byte 13
/// high nibble).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VsKind {
    /// Plain Vs. UniSystem.
    #[default]
    Uni,
    /// RBI Baseball: sequence read from $5E01.
    RbiBaseball,
    /// TKO Boxing: sequence read from $5E01.
    TkoBoxing,
    /// Super Xevious: four magic addresses in $5400-$57FF.
    SuperXevious,
    /// Vs. Ice Climber (Japan): Start lines held high.
    IceClimber,
    /// Vs. DualSystem: two linked boards.
    Dual,
    /// Raid on Bungeling Bay: DualSystem with Start lines held high.
    DualBungeling,
}

impl VsKind {
    pub fn from_nes2(nibble: u8) -> Self {
        match nibble & 0x0F {
            1 => VsKind::RbiBaseball,
            2 => VsKind::TkoBoxing,
            3 => VsKind::SuperXevious,
            4 => VsKind::IceClimber,
            5 => VsKind::Dual,
            6 => VsKind::DualBungeling,
            _ => VsKind::Uni,
        }
    }

    /// Two linked consoles.
    pub fn is_dual(self) -> bool {
        matches!(self, VsKind::Dual | VsKind::DualBungeling)
    }

    /// The game checks that the controllers' Start line reads 1.
    fn start_held(self) -> bool {
        matches!(self, VsKind::IceClimber | VsKind::DualBungeling)
    }
}

/// How the cabinet's controls reach the ports (NES 2.0 byte 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VsInput {
    /// Player 1 on $4016.
    #[default]
    Normal,
    /// Player 1 on $4017.
    Swapped,
    /// Vs. Pinball (Japan): P1's B and P2's A trade places.
    SwapAB,
    /// Vs. Zapper on $4016.
    Zapper,
}

impl VsInput {
    pub fn from_nes2(device: u8) -> Self {
        match device & 0x3F {
            5 => VsInput::Swapped,
            6 => VsInput::SwapAB,
            7 => VsInput::Zapper,
            _ => VsInput::Normal,
        }
    }
}

/// Everything that sets one Vs. game's hardware apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VsInfo {
    pub ppu: VsPpu,
    pub kind: VsKind,
    pub input: VsInput,
}

impl VsInfo {
    /// Route host button masks for players 1-2 to the two ports the way
    /// the cabinet is wired. Start and Select are swapped on every cabinet
    /// (its start buttons drive the Select line), but stay with their
    /// player when the ports are swapped.
    pub fn remap_buttons(&self, [p1, p2]: [u8; 2]) -> [u8; 2] {
        const SS: u8 = BTN_START | BTN_SELECT;
        let bit = |m: u8, b: u8, to: u8| if m & b != 0 { to } else { 0 };
        let (mut a, mut b) = match self.input {
            VsInput::Swapped => ((p2 & !SS) | (p1 & SS), (p1 & !SS) | (p2 & SS)),
            VsInput::SwapAB => (
                (p1 & !BTN_B) | bit(p2, BTN_A, BTN_B),
                (p2 & !BTN_A) | bit(p1, BTN_B, BTN_A),
            ),
            VsInput::Normal | VsInput::Zapper => (p1, p2),
        };
        for m in [&mut a, &mut b] {
            *m = (*m & !SS) | bit(*m, BTN_START, BTN_SELECT) | bit(*m, BTN_SELECT, BTN_START);
            if self.kind.start_held() {
                *m |= BTN_START;
            }
        }
        [a, b]
    }
}

/// Vs. board latches that are machine state (savestated with the bus).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VsLatch {
    /// Position in the RBI/TKO protection sequence.
    prot_index: u8,
    /// Super Xevious: which of its two answer sets is current.
    xevious: bool,
    /// Bit 1 of the last $4016 write. DualSystem: on the main board, 1
    /// gives it the shared RAM; on either board, 0 interrupts the other.
    pub main_sub: bool,
    /// Vs. Zapper serial shift register and strobe level.
    zapper_shift: u8,
    zapper_strobe: bool,
}

/// RBI Baseball's protection chip, as read from $5E01 (Mesen).
const PROT_RBI: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x6F, 0x00, 0x00, 0x00, 0x00, 0x94, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// TKO Boxing's protection chip, as read from $5E01 (MAME, Mesen).
const PROT_TKO: [u8; 32] = [
    0xFF, 0xBF, 0xB7, 0x97, 0x97, 0x17, 0x57, 0x4F, 0x6F, 0x6B, 0xEB, 0xA9, 0xB1, 0x90, 0x94, 0x14,
    0x56, 0x4E, 0x6F, 0x6B, 0xEB, 0xA9, 0xB1, 0x90, 0xD4, 0x5C, 0x3E, 0x26, 0x87, 0x83, 0x13, 0x00,
];

/// Coin-op inputs the host drives, read back through $4016/$4017, plus the
/// board's protection and Zapper latches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VsIo {
    pub coin1: bool,
    pub coin2: bool,
    pub service: bool,
    /// DIP switches 1-8 (bit 0 = switch 1).
    pub dip: u8,
    pub info: VsInfo,
    /// This board is a DualSystem's sub (second) console.
    pub is_sub: bool,
    pub latch: VsLatch,
}

impl VsIo {
    pub fn new(info: VsInfo) -> Self {
        VsIo {
            coin1: false,
            coin2: false,
            service: false,
            dip: 0,
            info,
            is_sub: false,
            latch: VsLatch::default(),
        }
    }

    /// Extra bits of a $4016 read (bit 0, the controller, is ORed in by
    /// the bus): service (2), DIP 1-2 (3-4), coin 1 (5), coin 2 (6), and
    /// 1 in bit 7 on a DualSystem's sub board.
    pub fn port1_bits(&self) -> u8 {
        (self.service as u8) << 2
            | (self.dip & 0x03) << 3
            | (self.coin1 as u8) << 5
            | (self.coin2 as u8) << 6
            | (self.is_sub as u8) << 7
    }

    /// Extra bits of a $4017 read: DIP switches 3-8 in bits 2-7.
    pub fn port2_bits(&self) -> u8 {
        self.dip & 0xFC
    }

    /// A $4020-$5FFF read the protection chip answers, if any.
    pub fn prot_read(&mut self, addr: u16) -> Option<u8> {
        let l = &mut self.latch;
        let mut seq = |table: &[u8; 32]| {
            let v = table[(l.prot_index & 0x1F) as usize];
            l.prot_index = l.prot_index.wrapping_add(1);
            v
        };
        match (self.info.kind, addr) {
            (VsKind::RbiBaseball | VsKind::TkoBoxing, 0x5E00) => {
                l.prot_index = 0;
                Some(0)
            }
            (VsKind::RbiBaseball, 0x5E01) => Some(seq(&PROT_RBI)),
            (VsKind::TkoBoxing, 0x5E01) => Some(seq(&PROT_TKO)),
            (VsKind::SuperXevious, 0x54FF) => Some(0x05),
            (VsKind::SuperXevious, 0x5678) => Some(if l.xevious { 0x00 } else { 0x01 }),
            (VsKind::SuperXevious, 0x578F) => Some(if l.xevious { 0xD1 } else { 0x89 }),
            (VsKind::SuperXevious, 0x5567) => {
                l.xevious = !l.xevious;
                Some(if l.xevious { 0x37 } else { 0x3E })
            }
            _ => None,
        }
    }

    /// Whether $4016 D0 comes from a Vs. Zapper.
    pub fn has_zapper(&self) -> bool {
        self.info.input == VsInput::Zapper
    }

    fn zapper_byte(light: bool, trigger: bool) -> u8 {
        0x10 | (light as u8) << 6 | (trigger as u8) << 7
    }

    /// A $4016 write: latch bit 1 and, with a Vs. Zapper, reload its shift
    /// register as the strobe falls. `light`/`trigger` are the gun's state
    /// now.
    pub fn write_4016(&mut self, val: u8, light: bool, trigger: bool) {
        self.latch.main_sub = val & 0x02 != 0;
        let strobe = val & 0x01 != 0;
        if self.latch.zapper_strobe && !strobe {
            self.latch.zapper_shift = Self::zapper_byte(light, trigger);
        }
        self.latch.zapper_strobe = strobe;
    }

    /// Vs. Zapper: next serial bit for $4016 D0 (reloading while strobed).
    pub fn zapper_read(&mut self, light: bool, trigger: bool) -> u8 {
        if self.latch.zapper_strobe {
            self.latch.zapper_shift = Self::zapper_byte(light, trigger);
        }
        let bit = self.latch.zapper_shift & 0x01;
        self.latch.zapper_shift >>= 1;
        bit
    }
}

/// CRC-32 (IEEE), as used to key the game database.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// Database entry for a ROM body (PRG+CHR, after header and trainer).
fn lookup(body: &[u8]) -> Option<VsInfo> {
    let crc = crc32(body);
    let i = crate::vs_db::VS_GAMES
        .binary_search_by_key(&crc, |e| e.0)
        .ok()?;
    let (_, kind, ppu, input) = crate::vs_db::VS_GAMES[i];
    Some(VsInfo {
        ppu: VsPpu::from_db(ppu),
        kind: VsKind::from_nes2(kind),
        input: VsInput::from_nes2(input),
    })
}

/// Vs. hardware a ROM image needs, if it is a Vs. dump: from the NES 2.0
/// header, else the game database, else the iNES Vs. flag with defaults.
pub fn detect(rom: &[u8]) -> Option<VsInfo> {
    if rom.len() < 16 || &rom[0..4] != b"NES\x1A" {
        return None;
    }
    let header = &rom[..16];
    let body = 16 + if header[6] & 0x04 != 0 { 512 } else { 0 };
    let db = rom.get(body..).and_then(lookup);
    if header[7] & 0x0C == 0x08 {
        // NES 2.0 console type 1 = Vs. System. Byte 13 names the PPU and
        // board, byte 15 the controls (0 = unspecified: ask the database).
        if header[7] & 0x03 != 1 {
            return None;
        }
        let input = match header[15] & 0x3F {
            0 => db.map_or(VsInput::Normal, |d| d.input),
            dev => VsInput::from_nes2(dev),
        };
        return Some(VsInfo {
            ppu: VsPpu::from_nes2(header[13]),
            kind: VsKind::from_nes2(header[13] >> 4),
            input,
        });
    }
    if db.is_some() {
        return db;
    }
    // Plain iNES: flags7 bit 0. Old dumps with junk in bytes 12-15
    // ("DiskDude!") can't be trusted to mean it.
    let clean = header[12..16].iter().all(|&b| b == 0);
    if !clean || header[7] & 0x01 == 0 {
        return None;
    }
    // Mapper 99 with two boards' worth of PRG is a DualSystem (as Mesen).
    let mapper = header[6] >> 4 | header[7] & 0xF0;
    let dual = mapper == 99 && header[4] >= 4;
    Some(VsInfo {
        ppu: VsPpu::Rp2c03,
        kind: if dual { VsKind::Dual } else { VsKind::Uni },
        input: VsInput::Normal,
    })
}

/// Split a Vs. DualSystem dump into one mapper-99 iNES image per board,
/// main first. The dump holds the main board's PRG then the sub's, and
/// likewise for CHR (8KB CHR images that fit one board are shared). Odd
/// layouts: 48KB of PRG is 24KB per board at $A000-$FFFF; Raid on Bungeling
/// Bay's sub board has a single 8KB bank after the main board's 32KB.
pub fn split_dual(rom: &[u8], kind: VsKind) -> Result<[Vec<u8>; 2], String> {
    const K8: usize = 0x2000;
    const K16: usize = 0x4000;
    let h = rom.get(..16).ok_or("ROM too short")?;
    let nes2 = h[7] & 0x0C == 0x08;
    let msb = |nibble: u8| if nes2 { (nibble as usize) << 8 } else { 0 };
    let prg_len = (h[4] as usize | msb(h[9] & 0x0F)) * K16;
    let chr_len = (h[5] as usize | msb(h[9] >> 4)) * K8;
    let start = 16 + if h[6] & 0x04 != 0 { 512 } else { 0 };
    let truncated = || "DualSystem ROM is truncated".to_string();
    let prg = rom.get(start..start + prg_len).ok_or_else(truncated)?;
    let chr = rom
        .get(start + prg_len..start + prg_len + chr_len)
        .ok_or_else(truncated)?;
    if prg.len() < 2 * K8 {
        return Err("DualSystem ROM has too little PRG for two boards".into());
    }
    let (main_prg, sub_prg) = if kind == VsKind::DualBungeling && prg.len() <= 0xC000 {
        (
            prg[..0x8000.min(prg.len())].to_vec(),
            prg[0x8000.min(prg.len() - K8)..][..K8].to_vec(),
        )
    } else if prg.len() == 0xC000 {
        // 24KB each, mapped high: pad an unused bank in front for $8000.
        let high = |p: &[u8]| [&[0xFF; K8][..], p].concat();
        (high(&prg[..0x6000]), high(&prg[0x6000..]))
    } else {
        let half = prg.len() / 2;
        (prg[..half].to_vec(), prg[half..].to_vec())
    };
    let (main_chr, sub_chr) = if chr.len() >= 2 * K16 {
        let half = chr.len() / 2;
        (chr[..half].to_vec(), chr[half..].to_vec())
    } else {
        (chr.to_vec(), chr.to_vec())
    };
    // Whole 16KB PRG units, repeating a short image (mirroring on hardware).
    let units = |mut p: Vec<u8>| {
        while !p.len().is_multiple_of(K16) {
            p.extend_from_within(..p.len().min(K16 - p.len() % K16));
        }
        p
    };
    let image = |prg: Vec<u8>, chr: Vec<u8>| {
        let prg = units(prg);
        let mut out = vec![0u8; 16];
        out[..4].copy_from_slice(b"NES\x1A");
        out[4] = (prg.len() / K16) as u8;
        out[5] = (chr.len() / K8) as u8;
        // Mapper 99; keep mirroring, battery and four-screen; Vs. flag.
        out[6] = 0x30 | (h[6] & 0x0B);
        out[7] = 0x61;
        out.extend_from_slice(&prg);
        out.extend_from_slice(&chr);
        out
    };
    Ok([image(main_prg, main_chr), image(sub_prg, sub_chr)])
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
        assert_eq!(detect(&header(0x01, 0)).map(|i| i.ppu), Some(VsPpu::Rp2c03));
        // Junk in bytes 12-15 voids the iNES Vs. bit.
        let mut junk = header(0x01, 0);
        junk[12..16].copy_from_slice(b"Dude");
        assert_eq!(detect(&junk), None);
        // NES 2.0 console type 1 with PPU types from byte 13.
        assert_eq!(
            detect(&header(0x09, 0x03)).map(|i| i.ppu),
            Some(VsPpu::Rp2c04(2))
        );
        assert_eq!(
            detect(&header(0x09, 0x06)).map(|i| i.ppu),
            Some(VsPpu::Rc2c03b)
        );
        assert_eq!(
            detect(&header(0x09, 0x09)).map(|i| i.ppu),
            Some(VsPpu::Rc2c05(2))
        );
        assert_eq!(
            detect(&header(0x09, 0x01)).map(|i| i.ppu),
            Some(VsPpu::Rp2c03)
        );
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

    fn info(kind: VsKind, input: VsInput) -> VsInfo {
        VsInfo {
            ppu: VsPpu::Rp2c03,
            kind,
            input,
        }
    }

    #[test]
    fn nes2_bytes_13_and_15_select_board_and_controls() {
        let mut h = header(0x09, 0x14); // RBI Baseball board, RP2C04-0003
        h[15] = 0x05;
        let i = detect(&h).unwrap();
        assert_eq!(
            (i.ppu, i.kind, i.input),
            (VsPpu::Rp2c04(3), VsKind::RbiBaseball, VsInput::Swapped)
        );
        let mut h = header(0x09, 0x50);
        h[15] = 0x07;
        let i = detect(&h).unwrap();
        assert!(i.kind.is_dual());
        assert_eq!(i.input, VsInput::Zapper);
        assert_eq!(VsKind::from_nes2(6), VsKind::DualBungeling);
        assert_eq!(VsKind::from_nes2(3), VsKind::SuperXevious);
    }

    #[test]
    fn ines_mapper_99_with_64kb_prg_is_dual() {
        let mut h = header(0x61, 0);
        h[6] = 0x30;
        h[4] = 4;
        assert_eq!(detect(&h).unwrap().kind, VsKind::Dual);
        h[4] = 2;
        assert_eq!(detect(&h).unwrap().kind, VsKind::Uni);
    }

    #[test]
    fn game_database_is_sorted_and_crc_is_ieee() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let db = &crate::vs_db::VS_GAMES;
        assert!(db.windows(2).all(|w| w[0].0 < w[1].0));
        // Vs. TKO Boxing (PRG+CHR CRC 52C501D0): TKO chip, RP2C04-0003.
        let i = db.binary_search_by_key(&0x52C5_01D0, |e| e.0).unwrap();
        let (_, kind, ppu, _) = db[i];
        assert_eq!(VsKind::from_nes2(kind), VsKind::TkoBoxing);
        assert_eq!(VsPpu::from_db(ppu), VsPpu::Rp2c04(3));
        assert_eq!(VsPpu::from_db(6), VsPpu::Rc2c05(1));
        assert_eq!(VsPpu::from_db(0), VsPpu::Rp2c03);
    }

    #[test]
    fn rbi_and_tko_sequences_reset_by_5e00() {
        let mut io = VsIo::new(info(VsKind::RbiBaseball, VsInput::Normal));
        assert_eq!(io.prot_read(0x5E00), Some(0));
        let seq: Vec<u8> = (0..15).map(|_| io.prot_read(0x5E01).unwrap()).collect();
        assert_eq!((seq[4], seq[9], seq[14], seq[0]), (0xB4, 0x6F, 0x94, 0x00));
        io.prot_read(0x5E00);
        assert_eq!(io.prot_read(0x5E01), Some(0x00));
        assert_eq!(io.prot_read(0x5000), None, "other addresses fall through");

        let mut io = VsIo::new(info(VsKind::TkoBoxing, VsInput::Normal));
        io.prot_read(0x5E00);
        assert_eq!(io.prot_read(0x5E01), Some(0xFF));
        assert_eq!(io.prot_read(0x5E01), Some(0xBF));
        // A plain board has no chip.
        let mut io = VsIo::new(info(VsKind::Uni, VsInput::Normal));
        assert_eq!(io.prot_read(0x5E01), None);
    }

    #[test]
    fn super_xevious_answers_alternate() {
        let mut io = VsIo::new(info(VsKind::SuperXevious, VsInput::Normal));
        let round =
            |io: &mut VsIo| [0x54FF, 0x5678, 0x578F, 0x5567].map(|a| io.prot_read(a).unwrap());
        assert_eq!(round(&mut io), [0x05, 0x01, 0x89, 0x37]);
        assert_eq!(round(&mut io), [0x05, 0x00, 0xD1, 0x3E]);
        assert_eq!(round(&mut io), [0x05, 0x01, 0x89, 0x37]);
    }

    #[test]
    fn buttons_follow_cabinet_wiring() {
        use crate::controller::{BTN_DOWN, BTN_UP};
        let (p1, p2) = (BTN_A | BTN_START | BTN_UP, BTN_B | BTN_SELECT | BTN_DOWN);
        // Start/Select always trade places.
        let n = info(VsKind::Uni, VsInput::Normal).remap_buttons([p1, p2]);
        assert_eq!(
            n,
            [BTN_A | BTN_SELECT | BTN_UP, BTN_B | BTN_START | BTN_DOWN]
        );
        // Swapped ports: the pads trade, start buttons stay with their side.
        let s = info(VsKind::Uni, VsInput::Swapped).remap_buttons([p1, p2]);
        assert_eq!(
            s,
            [BTN_B | BTN_SELECT | BTN_DOWN, BTN_A | BTN_START | BTN_UP]
        );
        // Pinball (J): P1 B <-> P2 A.
        let ab = info(VsKind::Uni, VsInput::SwapAB).remap_buttons([BTN_B, 0]);
        assert_eq!(ab, [0, BTN_A]);
        let ab = info(VsKind::Uni, VsInput::SwapAB).remap_buttons([0, BTN_A]);
        assert_eq!(ab, [BTN_B, 0]);
        // Ice Climber (J) protection: Start lines held.
        let ic = info(VsKind::IceClimber, VsInput::Normal).remap_buttons([0, 0]);
        assert_eq!(ic, [BTN_START, BTN_START]);
    }

    #[test]
    fn vs_zapper_shifts_out_a_latched_byte() {
        let mut io = VsIo::new(info(VsKind::Uni, VsInput::Zapper));
        assert!(io.has_zapper());
        io.write_4016(1, true, true);
        io.write_4016(0, true, true); // strobe falls: latch $D0
        let bits: Vec<u8> = (0..8).map(|_| io.zapper_read(false, false)).collect();
        assert_eq!(bits, [0, 0, 0, 0, 1, 0, 1, 1]);
        // Dark, trigger released: only the always-set D4.
        io.write_4016(1, false, false);
        io.write_4016(0, false, false);
        let byte = (0..8).fold(0u8, |acc, i| acc | io.zapper_read(true, true) << i);
        assert_eq!(byte, 0x10);
        // Bit 1 of the write is latched for the DualSystem link.
        io.write_4016(0x02, false, false);
        assert!(io.latch.main_sub);
    }

    /// iNES mapper-99 image: `prg_kb` of PRG where each 8KB bank holds its
    /// index, `chr_kb` of CHR filled per 8KB bank with $10 + index.
    fn dual_image(prg_kb: usize, chr_kb: usize) -> Vec<u8> {
        let mut h = header(0x61, 0);
        h[4] = (prg_kb / 16) as u8;
        h[5] = (chr_kb / 8) as u8;
        h[6] = 0x3A;
        h.extend((0..prg_kb * 1024).map(|i| (i / 0x2000) as u8));
        h.extend((0..chr_kb * 1024).map(|i| 0x10 + (i / 0x2000) as u8));
        h
    }

    /// (PRG 8KB bank markers, CHR 8KB bank markers) of a split image.
    fn banks(img: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let prg = img[4] as usize * 0x4000;
        assert_eq!(img[6] >> 4 | img[7] & 0xF0, 99);
        assert_eq!(img[6] & 0x0B, 0x0A, "four-screen + battery kept");
        let body = &img[16..];
        (
            body[..prg].iter().step_by(0x2000).copied().collect(),
            body[prg..].iter().step_by(0x2000).copied().collect(),
        )
    }

    #[test]
    fn dual_dumps_split_per_board() {
        let [m, s] = split_dual(&dual_image(64, 32), VsKind::Dual).unwrap();
        assert_eq!(banks(&m), (vec![0, 1, 2, 3], vec![0x10, 0x11]));
        assert_eq!(banks(&s), (vec![4, 5, 6, 7], vec![0x12, 0x13]));
        // 48KB: 24KB per board at $A000-$FFFF; 8KB CHR shared.
        let [m, s] = split_dual(&dual_image(48, 8), VsKind::Dual).unwrap();
        assert_eq!(banks(&m), (vec![0xFF, 0, 1, 2], vec![0x10]));
        assert_eq!(banks(&s), (vec![0xFF, 3, 4, 5], vec![0x10]));
        // Raid on Bungeling Bay: 32KB main, one 8KB sub bank mirrored.
        let [m, s] = split_dual(&dual_image(48, 16), VsKind::DualBungeling).unwrap();
        assert_eq!(banks(&m).0, vec![0, 1, 2, 3]);
        assert_eq!(banks(&s).0, vec![4, 4]);
        // Truncated dumps are rejected.
        let mut short = dual_image(64, 32);
        short.truncate(0x8000);
        assert!(split_dual(&short, VsKind::Dual).is_err());
    }

    #[test]
    fn coin_dip_service_bits() {
        let mut io = VsIo {
            coin1: true,
            service: true,
            dip: 0b1010_0110,
            ..VsIo::new(info(VsKind::Uni, VsInput::Normal))
        };
        assert_eq!(io.port1_bits(), 0x04 | (0b10 << 3) | 0x20);
        assert_eq!(io.port2_bits(), 0b1010_0100);
        // A DualSystem sub board reads 1 in bit 7.
        io.is_sub = true;
        assert_eq!(io.port1_bits() & 0x80, 0x80);
    }
}
