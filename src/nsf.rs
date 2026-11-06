//! NSF (NES Sound Format) support: header parsing and the public metadata
//! the frontend shows. An NSF is not a cartridge image but a music rip - a
//! block of 6502 code plus three entry points (load, init, play). Playback
//! runs that code on the normal emulated machine through a synthetic board,
//! [`crate::mapper::NsfMapper`], which banks the data, serves a tiny built-in
//! driver program and fires the play routine at the header's rate.
//!
//! Format reference: <https://www.nesdev.org/wiki/NSF>.

use crate::cartridge::Region;

/// Expansion-chip bits of header byte $7B.
pub const EXP_VRC6: u8 = 0x01;
pub const EXP_VRC7: u8 = 0x02;
pub const EXP_FDS: u8 = 0x04;
pub const EXP_MMC5: u8 = 0x08;
pub const EXP_N163: u8 = 0x10;
pub const EXP_5B: u8 = 0x20;

/// Size of the fixed NSF header.
pub const HEADER_LEN: usize = 0x80;

/// Default play-routine periods in microseconds when the header leaves the
/// speed field 0 (the nominal 60.0988 Hz / 50.007 Hz frame rates).
pub const DEFAULT_NTSC_US: u16 = 16_639;
pub const DEFAULT_PAL_US: u16 = 19_997;

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
}

/// Song metadata reported to the frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsfInfo {
    pub title: String,
    pub artist: String,
    pub copyright: String,
    pub total_songs: usize,
    /// Currently selected song, 0-based.
    pub current_song: usize,
    pub expansion: u8,
}

/// True when `data` starts with the NSF magic.
pub fn is_nsf(data: &[u8]) -> bool {
    data.len() >= 5 && &data[0..5] == b"NESM\x1A"
}

/// Decode a 32-byte NUL-padded header string. NSF text is nominally ASCII;
/// non-ASCII bytes (Shift-JIS rips) are replaced so the UI font can draw it.
fn header_str(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..end]
        .iter()
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                '?'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Parse the header and return it with the program data that follows (NSF2
/// trailing metadata, if declared, is cut off).
pub fn parse(data: &[u8]) -> Result<(NsfHeader, &[u8]), String> {
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
    let load_addr = word(0x08);
    let init_addr = word(0x0A);
    let play_addr = word(0x0C);
    let mut bank_init = [0u8; 8];
    bank_init.copy_from_slice(&h[0x70..0x78]);
    let bankswitched = bank_init.iter().any(|&b| b != 0);
    // FDS tunes run from RAM and may load as low as $6000.
    let min_load = if h[0x7B] & EXP_FDS != 0 {
        0x6000
    } else {
        0x8000
    };
    if !bankswitched && load_addr < min_load {
        return Err(format!(
            "NSF load address ${load_addr:04X} below ${min_load:04X}"
        ));
    }
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
    if h[0x05] >= 2 {
        let len = h[0x7D] as usize | (h[0x7E] as usize) << 8 | (h[0x7F] as usize) << 16;
        if len > 0 && len < body.len() {
            body = &body[..len];
        }
    }
    let header = NsfHeader {
        version: h[0x05],
        total_songs,
        start_song: (h[0x07] as usize).clamp(1, total_songs) - 1,
        load_addr,
        init_addr,
        play_addr,
        title: header_str(&h[0x0E..0x2E]),
        artist: header_str(&h[0x2E..0x4E]),
        copyright: header_str(&h[0x4E..0x6E]),
        speed_ntsc_us: speed(0x6E, DEFAULT_NTSC_US),
        speed_pal_us: speed(0x78, DEFAULT_PAL_US),
        bank_init,
        bankswitched,
        region,
        expansion: h[0x7B] & 0x3F,
    };
    Ok((header, body))
}

#[cfg(test)]
mod tests {
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
}
