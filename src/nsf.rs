//! NSF (NES Sound Format) support: header parsing and the public metadata
//! the frontend shows. An NSF is not a cartridge image but a music rip - a
//! block of 6502 code plus three entry points (load, init, play). Playback
//! runs that code on the normal emulated machine through a synthetic board,
//! [`crate::mapper::NsfMapper`], which banks the data, serves a tiny built-in
//! driver program and fires the play routine at the header's rate.
//!
//! Three containers are read:
//!
//! - **NSF** (`NESM\x1A`, version 1): a fixed 128-byte header.
//! - **NSF2** (same header, version 2): a feature-flags byte at $7C (IRQ
//!   timer, non-returning init, suppressed play) and a program length at
//!   $7D-$7F after which NSFe-style metadata chunks may follow.
//! - **NSFe** (`NSFE`): everything in chunks (`INFO`, `DATA`, `BANK`,
//!   `RATE`, `NSF2`, plus the optional metadata ones).
//!
//! Metadata chunks understood: `auth` (title/artist/copyright/ripper),
//! `tlbl` (track labels), `taut` (track authors), `time` / `fade` (track
//! lengths in ms), `plst` (playlist order), `regn` (region), `RATE` (play
//! speeds). Text is UTF-8 (NSF v1 header strings fall back to
//! Windows-1252) and is transliterated to the ASCII the UI font draws.
//!
//! Format references: <https://www.nesdev.org/wiki/NSF>,
//! <https://www.nesdev.org/wiki/NSF2>, <https://www.nesdev.org/wiki/NSFe>.

use crate::cartridge::Region;

/// Expansion-chip bits of header byte $7B.
pub const EXP_VRC6: u8 = 0x01;
pub const EXP_VRC7: u8 = 0x02;
pub const EXP_FDS: u8 = 0x04;
pub const EXP_MMC5: u8 = 0x08;
pub const EXP_N163: u8 = 0x10;
pub const EXP_5B: u8 = 0x20;

/// NSF2 feature flags (header byte $7C / NSFe `NSF2` chunk).
/// The tune may use the $401B-$401D IRQ timer and its own IRQ vector.
pub const FLAG_IRQ: u8 = 0x10;
/// Init is called a second time (Y = $81) and need not return; play runs
/// from NMI.
pub const FLAG_NON_RETURNING_INIT: u8 = 0x20;
/// Play is never called.
pub const FLAG_SUPPRESS_PLAY: u8 = 0x40;
/// The metadata may hold chunks required for playback.
pub const FLAG_MANDATORY_META: u8 = 0x80;

/// Size of the fixed NSF header.
pub const HEADER_LEN: usize = 0x80;

/// Default play-routine periods in microseconds when the header leaves the
/// speed field 0 (the nominal 60.0988 Hz / 50.007 Hz frame rates).
pub const DEFAULT_NTSC_US: u16 = 16_639;
pub const DEFAULT_PAL_US: u16 = 19_997;

/// Optional NSFe-style metadata (NSF2 trailer or NSFe chunks). Per-track
/// vectors are indexed by 0-based song number and may be shorter than the
/// song count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NsfMeta {
    pub ripper: String,
    pub labels: Vec<String>,
    pub authors: Vec<String>,
    /// Track lengths in ms; None = unknown (play forever).
    pub times: Vec<Option<u32>>,
    /// Fade-out lengths in ms; None = player default.
    pub fades: Vec<Option<u32>>,
    /// Play order (0-based songs); empty = 0..total_songs.
    pub playlist: Vec<usize>,
}

/// Parsed NSF header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsfHeader {
    pub version: u8,
    pub total_songs: usize,
    /// First song to play, 0-based (the file stores it 1-based).
    pub start_song: usize,
    pub load_addr: u16,
    pub init_addr: u16,
    pub play_addr: u16,
    pub title: String,
    pub artist: String,
    pub copyright: String,
    /// Play-routine period in microseconds for each region (never 0).
    pub speed_ntsc_us: u16,
    pub speed_pal_us: u16,
    /// Initial 4KB bank numbers for $8000-$FFFF.
    pub bank_init: [u8; 8],
    /// True when any bank-init byte is non-zero: the tune uses $5FF8-$5FFF.
    pub bankswitched: bool,
    pub region: Region,
    pub expansion: u8,
    /// NSF2 feature flags (`FLAG_*`); 0 for version 1 files.
    pub flags: u8,
    pub meta: NsfMeta,
}

/// Song metadata reported to the frontend.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NsfInfo {
    pub title: String,
    pub artist: String,
    pub copyright: String,
    pub total_songs: usize,
    /// Currently selected song, 0-based.
    pub current_song: usize,
    pub expansion: u8,
    /// Current track's label and author (empty when the file has none).
    pub track_label: String,
    pub track_author: String,
    /// Current track's length and fade-out in ms, when the file says.
    pub track_ms: Option<u32>,
    pub fade_ms: Option<u32>,
    /// Play order (0-based songs); empty = natural order.
    pub playlist: Vec<usize>,
}

/// Fade-out used when a track has a length but no `fade` entry.
pub const DEFAULT_FADE_MS: u32 = 2_000;

impl NsfInfo {
    /// Song `step` places away from the current one in play order, wrapping
    /// around (`step` = 1 next, -1 previous). Follows the playlist when the
    /// file has one; a current song not on it restarts the list.
    pub fn neighbour(&self, step: isize) -> usize {
        let order: Vec<usize> = if self.playlist.is_empty() {
            (0..self.total_songs.max(1)).collect()
        } else {
            self.playlist.clone()
        };
        let n = order.len() as isize;
        match order.iter().position(|&s| s == self.current_song) {
            Some(i) => order[(i as isize + step).rem_euclid(n) as usize],
            None => order[0],
        }
    }

    /// Total play time of the current track (length + fade) in ms, or None
    /// when it plays until the user changes it.
    pub fn track_total_ms(&self) -> Option<u32> {
        self.track_ms
            .map(|t| t.saturating_add(self.fade_ms.unwrap_or(DEFAULT_FADE_MS)))
    }

    /// Output gain `elapsed_ms` into the current track: 1.0 until its
    /// length, then a linear fade to 0 over the fade time.
    pub fn gain_at(&self, elapsed_ms: u64) -> f32 {
        let Some(t) = self.track_ms else {
            return 1.0;
        };
        let fade = self.fade_ms.unwrap_or(DEFAULT_FADE_MS) as u64;
        let past = elapsed_ms.saturating_sub(t as u64);
        if past == 0 {
            1.0
        } else if past >= fade {
            0.0
        } else {
            1.0 - past as f32 / fade as f32
        }
    }
}

/// True when `data` starts with the NSF or NSFe magic.
pub fn is_nsf(data: &[u8]) -> bool {
    (data.len() >= 5 && &data[0..5] == b"NESM\x1A") || data.starts_with(b"NSFE")
}

/// Windows-1252 code points for bytes $80-$9F (0 = undefined).
const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0, 0x017D, 0, 0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC,
    0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
];

/// Decode text bytes: UTF-8 when valid, otherwise Windows-1252 (a superset
/// of Latin-1 that most European v1 rips use).
fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes
            .iter()
            .map(|&b| match b {
                0x80..=0x9F => char::from_u32(CP1252_HIGH[b as usize - 0x80] as u32)
                    .filter(|&c| c != '\0')
                    .unwrap_or('?'),
                _ => b as char,
            })
            .collect(),
    }
}

/// Base letters for U+0100-U+017F (Latin Extended-A), in code point order.
const LATIN_EXT_A: &str = concat!(
    "AaAaAa",
    "CcCcCcCc",
    "DdDd",
    "EeEeEeEeEe",
    "GgGgGgGg",
    "HhHh",
    "IiIiIiIiIi",
    "Ii",
    "Jj",
    "Kkk",
    "LlLlLlLlLl",
    "NnNnNnnNn",
    "OoOoOo",
    "Oo",
    "RrRrRr",
    "SsSsSsSs",
    "TtTtTt",
    "UuUuUuUuUuUu",
    "Ww",
    "YyY",
    "ZzZzZz",
    "s",
);

/// ASCII stand-in for one character, or None when there is none.
fn ascii_for(c: char) -> Option<&'static str> {
    // Latin-1 letters, U+00C0-U+00FF.
    const LATIN1: [&str; 64] = [
        "A", "A", "A", "A", "A", "A", "AE", "C", "E", "E", "E", "E", "I", "I", "I", "I", "D", "N",
        "O", "O", "O", "O", "O", "x", "O", "U", "U", "U", "U", "Y", "TH", "ss", "a", "a", "a", "a",
        "a", "a", "ae", "c", "e", "e", "e", "e", "i", "i", "i", "i", "d", "n", "o", "o", "o", "o",
        "o", "/", "o", "u", "u", "u", "u", "y", "th", "y",
    ];
    let cp = c as u32;
    Some(match cp {
        0xC0..=0xFF => LATIN1[cp as usize - 0xC0],
        0x100..=0x17F => {
            let i = cp as usize - 0x100;
            return LATIN_EXT_A.get(i..i + 1);
        }
        0xA0 | 0x3000 => " ",
        0xA9 => "(C)",
        0xAE => "(R)",
        0x2122 => "TM",
        0xB0 => "o",
        0xB7 | 0x2022 | 0x30FB => "*",
        0x2018 | 0x2019 | 0x201A | 0x201B | 0xB4 | 0x60 => "'",
        0x201C | 0x201D | 0x201E | 0xAB | 0xBB => "\"",
        0x2010..=0x2015 | 0x30FC => "-",
        0x2026 => "...",
        0x301C | 0xFF5E => "~",
        _ => return None,
    })
}

/// Make `s` drawable by the ASCII-only UI font: printable ASCII passes,
/// accented Latin letters lose their accents, fullwidth forms narrow,
/// typographic punctuation becomes its ASCII look-alike, control characters
/// become spaces and anything else (kana, kanji...) becomes '?', one per
/// run.
pub fn to_display_ascii(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let cp = c as u32;
        if (0x20..0x7F).contains(&cp) {
            out.push(c);
        } else if (0xFF01..=0xFF5E).contains(&cp) {
            // Fullwidth ASCII block.
            out.push(char::from_u32(cp - 0xFEE0).unwrap_or('?'));
        } else if c.is_control() {
            out.push(' ');
        } else if let Some(a) = ascii_for(c) {
            out.push_str(a);
        } else if !out.ends_with('?') {
            out.push('?');
        }
    }
    out.trim().to_string()
}

/// Decode a 32-byte NUL-padded header string.
fn header_str(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    to_display_ascii(&decode(&bytes[..end]))
}

/// Split a chunk of NUL-terminated strings.
fn strings(data: &[u8]) -> Vec<String> {
    let data = data.strip_suffix(&[0]).unwrap_or(data);
    if data.is_empty() {
        return Vec::new();
    }
    data.split(|&b| b == 0)
        .map(|s| to_display_ascii(&decode(s)))
        .collect()
}

/// Little-endian i32 list (`time` / `fade`); negative = unknown.
fn millis(data: &[u8]) -> Vec<Option<u32>> {
    data.as_chunks::<4>()
        .0
        .iter()
        .map(|&b| u32::try_from(i32::from_le_bytes(b)).ok())
        .collect()
}

/// Iterate `(fourcc, data)` chunks up to `NEND` or the end of the input. A
/// truncated chunk ends the walk.
fn chunks(mut data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        if data.len() < 8 {
            return None;
        }
        let len = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let id: [u8; 4] = data[4..8].try_into().unwrap();
        let body = data.get(8..8usize.checked_add(len)?)?;
        data = &data[8 + len..];
        (&id != b"NEND").then_some((id, body))
    })
}

/// Fields shared by NSF2 trailers and NSFe files that change playback.
#[derive(Default)]
struct Extra {
    meta: NsfMeta,
    auth: Vec<String>,
    /// `RATE`: NTSC, PAL, Dendy speeds (us; 0 = default).
    rate: Option<[u16; 3]>,
    /// `regn`: region to play in.
    region: Option<Region>,
}

/// Apply one metadata chunk. Returns false for an unknown mandatory
/// (uppercase) chunk, which voids the metadata.
fn metadata_chunk(x: &mut Extra, id: &[u8; 4], d: &[u8]) -> bool {
    match id {
        b"auth" => x.auth = strings(d),
        b"tlbl" => x.meta.labels = strings(d),
        b"taut" => x.meta.authors = strings(d),
        b"time" => x.meta.times = millis(d),
        b"fade" => x.meta.fades = millis(d),
        b"plst" => x.meta.playlist = d.iter().map(|&b| b as usize).collect(),
        b"RATE" => {
            let w = |i: usize| {
                d.get(i..i + 2)
                    .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
            };
            let pal = w(2);
            x.rate = Some([w(0), pal, if d.len() >= 6 { w(4) } else { pal }]);
        }
        b"regn" => {
            let bits = d.first().copied().unwrap_or(1);
            let pick = match d.get(1) {
                Some(&p) if p <= 2 && bits & (1 << p) != 0 => p,
                _ => (0..3).find(|i| bits & (1 << i) != 0).unwrap_or(0),
            };
            x.region = Some(match pick {
                1 => Region::Pal,
                2 => Region::Dendy,
                _ => Region::Ntsc,
            });
        }
        // Custom VRC7 patches: understood as "allowed", the default patch
        // set is used.
        b"VRC7" => {}
        _ if id[0].is_ascii_uppercase() => return false,
        _ => {}
    }
    true
}

/// Parse NSF2/NSFe metadata chunks; an unknown mandatory chunk drops the
/// whole metadata (playback goes on with the header alone).
fn parse_metadata(data: &[u8]) -> Extra {
    let mut x = Extra::default();
    for (id, d) in chunks(data) {
        if !metadata_chunk(&mut x, &id, d) {
            return Extra::default();
        }
    }
    x
}

/// Fold metadata into a parsed header.
fn apply_extra(h: &mut NsfHeader, x: Extra) {
    let pick = |s: Option<&String>, old: &mut String| {
        if let Some(s) = s.filter(|s| !s.is_empty() && s.as_str() != "<?>") {
            *old = s.clone();
        }
    };
    pick(x.auth.first(), &mut h.title);
    pick(x.auth.get(1), &mut h.artist);
    pick(x.auth.get(2), &mut h.copyright);
    if let Some(r) = x.auth.get(3) {
        h.meta.ripper = r.clone();
    }
    if let Some([ntsc, pal, _]) = x.rate {
        if ntsc != 0 {
            h.speed_ntsc_us = ntsc;
        }
        if pal != 0 {
            h.speed_pal_us = pal;
        }
    }
    if let Some(r) = x.region {
        h.region = r;
    }
    let ripper = std::mem::take(&mut h.meta.ripper);
    h.meta = NsfMeta { ripper, ..x.meta };
    let n = h.total_songs;
    h.meta.playlist.retain(|&s| s < n);
}

/// Parse the header and return it with the program data that follows (NSF2
/// trailing metadata, if declared, is cut off and parsed). NSFe files are
/// accepted too.
pub fn parse(data: &[u8]) -> Result<(NsfHeader, &[u8]), String> {
    if data.starts_with(b"NSFE") {
        return parse_nsfe(data);
    }
    if !is_nsf(data) {
        return Err("not an NSF file (bad magic)".into());
    }
    if data.len() <= HEADER_LEN {
        return Err("NSF file truncated (no program data)".into());
    }
    let h = &data[..HEADER_LEN];
    let word = |o: usize| u16::from_le_bytes([h[o], h[o + 1]]);
    let total_songs = h[0x06] as usize;
    if total_songs == 0 {
        return Err("NSF declares no songs".into());
    }
    let mut bank_init = [0u8; 8];
    bank_init.copy_from_slice(&h[0x70..0x78]);
    let speed = |o: usize, default: u16| match word(o) {
        0 => default,
        v => v,
    };
    // Bit 0 = PAL; bit 1 = dual-region, which we play as NTSC.
    let region = if h[0x7A] & 0x03 == 0x01 {
        Region::Pal
    } else {
        Region::Ntsc
    };
    let mut body = &data[HEADER_LEN..];
    // NSF2: bytes $7D-$7F give the program length when metadata follows it.
    // (Version 1 rips sometimes carry junk there, so they're not trusted.)
    let len = h[0x7D] as usize | (h[0x7E] as usize) << 8 | (h[0x7F] as usize) << 16;
    let mut extra = Extra::default();
    if h[0x05] >= 2 && len > 0 && len < body.len() {
        extra = parse_metadata(&body[len..]);
        body = &body[..len];
    }
    let mut header = NsfHeader {
        version: h[0x05],
        total_songs,
        start_song: (h[0x07] as usize).clamp(1, total_songs) - 1,
        load_addr: word(0x08),
        init_addr: word(0x0A),
        play_addr: word(0x0C),
        title: header_str(&h[0x0E..0x2E]),
        artist: header_str(&h[0x2E..0x4E]),
        copyright: header_str(&h[0x4E..0x6E]),
        speed_ntsc_us: speed(0x6E, DEFAULT_NTSC_US),
        speed_pal_us: speed(0x78, DEFAULT_PAL_US),
        bank_init,
        bankswitched: bank_init.iter().any(|&b| b != 0),
        region,
        expansion: h[0x7B] & 0x3F,
        // Version 1 players must ignore $7C.
        flags: if h[0x05] >= 2 { h[0x7C] & 0xF0 } else { 0 },
        meta: NsfMeta::default(),
    };
    apply_extra(&mut header, extra);
    validate(&header)?;
    Ok((header, body))
}

/// Checks shared by every container.
fn validate(h: &NsfHeader) -> Result<(), String> {
    // FDS tunes run from RAM and may load as low as $6000.
    let min_load = if h.expansion & EXP_FDS != 0 {
        0x6000
    } else {
        0x8000
    };
    if !h.bankswitched && h.load_addr < min_load {
        return Err(format!(
            "NSF load address ${:04X} below ${min_load:04X}",
            h.load_addr
        ));
    }
    Ok(())
}

/// Parse an NSFe file: `INFO` and `DATA` are required; `BANK`, `RATE`,
/// `NSF2` and the metadata chunks are optional.
fn parse_nsfe(data: &[u8]) -> Result<(NsfHeader, &[u8]), String> {
    let mut info = None;
    let mut body = None;
    let mut bank_init = [0u8; 8];
    let mut flags = 0;
    let mut x = Extra::default();
    for (id, d) in chunks(&data[4..]) {
        match &id {
            b"INFO" => info = Some(d),
            b"DATA" => body = Some(d),
            b"BANK" => {
                let n = d.len().min(8);
                bank_init[..n].copy_from_slice(&d[..n]);
            }
            b"NSF2" => flags = d.first().copied().unwrap_or(0) & 0xF0,
            _ => {
                if !metadata_chunk(&mut x, &id, d) {
                    let name = String::from_utf8_lossy(&id).into_owned();
                    return Err(format!("NSFe needs unsupported chunk {name}"));
                }
            }
        }
    }
    let i = info.ok_or("NSFe has no INFO chunk")?;
    let body = body.ok_or("NSFe has no DATA chunk")?;
    if i.len() < 9 {
        return Err("NSFe INFO chunk too short".into());
    }
    let word = |o: usize| u16::from_le_bytes([i[o], i[o + 1]]);
    let total_songs = i[8] as usize;
    if total_songs == 0 {
        return Err("NSF declares no songs".into());
    }
    let mut header = NsfHeader {
        version: 2,
        total_songs,
        // NSFe stores the starting song 0-based.
        start_song: (i.get(9).copied().unwrap_or(0) as usize).min(total_songs - 1),
        load_addr: word(0),
        init_addr: word(2),
        play_addr: word(4),
        title: String::new(),
        artist: String::new(),
        copyright: String::new(),
        speed_ntsc_us: DEFAULT_NTSC_US,
        speed_pal_us: DEFAULT_PAL_US,
        bank_init,
        bankswitched: bank_init.iter().any(|&b| b != 0),
        region: if i[6] & 0x03 == 0x01 {
            Region::Pal
        } else {
            Region::Ntsc
        },
        expansion: i[7] & 0x3F,
        flags,
        meta: NsfMeta::default(),
    };
    apply_extra(&mut header, x);
    validate(&header)?;
    Ok((header, body))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn header(songs: u8, start: u8) -> Vec<u8> {
        let mut h = vec![0u8; HEADER_LEN];
        h[0..5].copy_from_slice(b"NESM\x1A");
        h[5] = 1;
        h[6] = songs;
        h[7] = start;
        h[0x08..0x0A].copy_from_slice(&0x8000u16.to_le_bytes());
        h[0x0A..0x0C].copy_from_slice(&0x8000u16.to_le_bytes());
        h[0x0C..0x0E].copy_from_slice(&0x8003u16.to_le_bytes());
        h[0x0E..0x13].copy_from_slice(b"TITLE");
        h[0x2E..0x34].copy_from_slice(b"ARTIST");
        h[0x4E..0x52].copy_from_slice(b"2026");
        h
    }

    /// One NSFe-style chunk.
    pub(crate) fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = (data.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(id);
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn parses_good_header() {
        let mut data = header(5, 2);
        data[0x7B] = EXP_VRC6 | EXP_N163;
        data[0x6E..0x70].copy_from_slice(&8000u16.to_le_bytes());
        data.extend_from_slice(&[0xEA; 16]);
        let (h, body) = parse(&data).unwrap();
        assert_eq!(h.total_songs, 5);
        assert_eq!(h.start_song, 1);
        assert_eq!(h.load_addr, 0x8000);
        assert_eq!(h.play_addr, 0x8003);
        assert_eq!(h.title, "TITLE");
        assert_eq!(h.artist, "ARTIST");
        assert_eq!(h.copyright, "2026");
        assert_eq!(h.speed_ntsc_us, 8000);
        assert_eq!(h.speed_pal_us, DEFAULT_PAL_US);
        assert_eq!(h.region, Region::Ntsc);
        assert_eq!(h.expansion, EXP_VRC6 | EXP_N163);
        assert_eq!(h.flags, 0);
        assert_eq!(h.meta, NsfMeta::default());
        assert!(!h.bankswitched);
        assert_eq!(body.len(), 16);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(b"NES\x1A").is_err());
        let mut data = header(1, 1);
        assert!(parse(&data).is_err(), "header-only file has no program");
        data[0] = b'X';
        data.push(0);
        assert!(parse(&data).is_err(), "bad magic");
        let mut zero = header(0, 1);
        zero.push(0);
        assert!(parse(&zero).is_err(), "zero songs");
        let mut low = header(1, 1);
        low[0x08..0x0A].copy_from_slice(&0x6000u16.to_le_bytes());
        low.push(0);
        assert!(parse(&low).is_err(), "unbanked load below $8000");
    }

    #[test]
    fn region_and_nsf2_length() {
        let mut data = header(1, 1);
        data[0x7A] = 0x01;
        data[5] = 2;
        data[0x7D] = 4; // program is 4 bytes, metadata follows
        data.extend_from_slice(&[1, 2, 3, 4, 9, 9, 9]);
        let (h, body) = parse(&data).unwrap();
        assert_eq!(h.region, Region::Pal);
        assert_eq!(body, &[1, 2, 3, 4]);
        data[0x7A] = 0x03; // dual -> NTSC
        assert_eq!(parse(&data).unwrap().0.region, Region::Ntsc);
    }

    /// NSF2 file with 3 songs, a 4-byte program and `meta` appended.
    fn nsf2(flags: u8, meta: &[u8]) -> Vec<u8> {
        let mut data = header(3, 1);
        data[5] = 2;
        data[0x7C] = flags;
        data[0x7D] = 4;
        data.extend_from_slice(&[0xEA, 0xEA, 0xEA, 0x60]);
        data.extend_from_slice(meta);
        data
    }

    #[test]
    fn nsf2_metadata_chunks() {
        let mut meta = chunk(b"auth", "Gämé\0Ärtist\0(c) 1990\0Rippér\0".as_bytes());
        meta.extend(chunk(b"tlbl", b"Intro\0Stage 1\0"));
        meta.extend(chunk(b"taut", b"\0Guest\0"));
        let mut times = 90_000i32.to_le_bytes().to_vec();
        times.extend_from_slice(&(-1i32).to_le_bytes());
        times.extend_from_slice(&5_000i32.to_le_bytes());
        meta.extend(chunk(b"time", &times));
        meta.extend(chunk(b"fade", &1_000i32.to_le_bytes()));
        meta.extend(chunk(b"plst", &[2, 0, 7]));
        meta.extend(chunk(b"xtra", b"ignored optional chunk"));
        meta.extend(chunk(b"NEND", &[]));
        meta.extend(chunk(b"tlbl", b"after NEND: not read\0"));
        let file = nsf2(FLAG_IRQ | 0x0F, &meta);
        let (h, body) = parse(&file).unwrap();
        assert_eq!(body.len(), 4);
        assert_eq!(h.flags, FLAG_IRQ, "reserved bits dropped");
        assert_eq!(h.title, "Game");
        assert_eq!(h.artist, "Artist");
        assert_eq!(h.copyright, "(c) 1990");
        assert_eq!(h.meta.ripper, "Ripper");
        assert_eq!(h.meta.labels, ["Intro", "Stage 1"]);
        assert_eq!(h.meta.authors, ["", "Guest"]);
        assert_eq!(h.meta.times, [Some(90_000), None, Some(5_000)]);
        assert_eq!(h.meta.fades, [Some(1_000)]);
        assert_eq!(h.meta.playlist, [2, 0], "out-of-range entries dropped");
    }

    #[test]
    fn unknown_mandatory_chunk_voids_metadata_only() {
        let mut meta = chunk(b"tlbl", b"Intro\0");
        meta.extend(chunk(b"ZZZZ", &[1]));
        let file = nsf2(FLAG_MANDATORY_META, &meta);
        let (h, body) = parse(&file).unwrap();
        assert_eq!(h.meta, NsfMeta::default());
        assert_eq!(h.title, "TITLE");
        assert_eq!(body.len(), 4, "program still plays");
        // RATE and regn are understood.
        let mut meta = chunk(b"RATE", &[0x10, 0x27, 0x20, 0x4E]);
        meta.extend(chunk(b"regn", &[0b110, 2]));
        let (h, _) = parse(&nsf2(0, &meta)).unwrap();
        assert_eq!((h.speed_ntsc_us, h.speed_pal_us), (10_000, 20_000));
        assert_eq!(h.region, Region::Dendy);
    }

    #[test]
    fn v1_ignores_flags_and_decodes_latin1() {
        let mut data = header(1, 1);
        data[0x7C] = FLAG_IRQ;
        data[0x0E..0x14].copy_from_slice(b"Caf\xE9 \x93");
        data.push(0x60);
        let (h, _) = parse(&data).unwrap();
        assert_eq!(h.flags, 0);
        assert_eq!(h.title, "Cafe \"");
    }

    #[test]
    fn display_ascii_transliteration() {
        assert_eq!(LATIN_EXT_A.chars().count(), 0x80);
        assert_eq!(to_display_ascii("Ærøskøbing ŁÓDŹ"), "AEroskobing LODZ");
        assert_eq!(to_display_ascii("ＡＢＣ１２３"), "ABC123");
        assert_eq!(to_display_ascii("ロックマン 2"), "? 2");
        assert_eq!(to_display_ascii("“Hi”—you…"), "\"Hi\"-you...");
        assert_eq!(to_display_ascii("a\tb\n"), "a b");
        assert_eq!(header_str(b"OK\0junk"), "OK");
    }

    #[test]
    fn nsfe_container() {
        let mut info = Vec::new();
        for w in [0x8000u16, 0x8000, 0x8003] {
            info.extend_from_slice(&w.to_le_bytes());
        }
        info.extend_from_slice(&[0x01, EXP_VRC6, 4, 2]);
        let mut data = b"NSFE".to_vec();
        data.extend(chunk(b"INFO", &info));
        data.extend(chunk(b"BANK", &[0, 1]));
        data.extend(chunk(b"NSF2", &[FLAG_NON_RETURNING_INIT]));
        data.extend(chunk(b"DATA", &[0x60; 8]));
        data.extend(chunk(b"auth", b"Tune\0Me\0"));
        data.extend(chunk(b"NEND", &[]));
        assert!(is_nsf(&data));
        let (h, body) = parse(&data).unwrap();
        assert_eq!(body, &[0x60; 8]);
        assert_eq!(h.total_songs, 4);
        assert_eq!(h.start_song, 2);
        assert_eq!(h.region, Region::Pal);
        assert_eq!(h.expansion, EXP_VRC6);
        assert!(h.bankswitched);
        assert_eq!(h.bank_init, [0, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(h.flags, FLAG_NON_RETURNING_INIT);
        assert_eq!((h.title.as_str(), h.artist.as_str()), ("Tune", "Me"));
        // Missing DATA and unknown mandatory chunks are errors here.
        let mut bad = b"NSFE".to_vec();
        bad.extend(chunk(b"INFO", &info));
        assert!(parse(&bad).is_err());
        bad.extend(chunk(b"DATA", &[0x60]));
        bad.extend(chunk(b"QQQQ", &[]));
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn playlist_navigation_and_fade() {
        let mut info = NsfInfo {
            total_songs: 4,
            current_song: 3,
            ..Default::default()
        };
        assert_eq!(info.neighbour(1), 0);
        assert_eq!(info.neighbour(-1), 2);
        info.playlist = vec![3, 1];
        assert_eq!(info.neighbour(1), 1);
        assert_eq!(info.neighbour(-1), 1);
        info.current_song = 0; // not on the list
        assert_eq!(info.neighbour(1), 3);

        assert_eq!(info.gain_at(1_000_000), 1.0, "no length: no fade");
        assert_eq!(info.track_total_ms(), None);
        info.track_ms = Some(10_000);
        info.fade_ms = Some(2_000);
        assert_eq!(info.track_total_ms(), Some(12_000));
        assert_eq!(info.gain_at(9_000), 1.0);
        assert!((info.gain_at(11_000) - 0.5).abs() < 1e-6);
        assert_eq!(info.gain_at(12_000), 0.0);
        info.fade_ms = None;
        assert_eq!(info.track_total_ms(), Some(10_000 + DEFAULT_FADE_MS));
    }
}
