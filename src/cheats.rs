//! Cheat codes: Game Genie (6- and 8-letter) codes and raw address patches.
//!
//! Accepted forms (case-insensitive, surrounding whitespace ignored):
//! - `SXIOPO` / `YEUZUGAA` - Game Genie. Patches CPU reads of $8000-$FFFF,
//!   the 8-letter form only when the ROM byte equals the compare value.
//! - `AAAA:VV` - raw patch. In $8000-$FFFF it behaves like a Game Genie code;
//!   in RAM ($0000-$1FFF, $6000-$7FFF) the value is forced every frame
//!   (Pro Action Replay style).
//! - `AAAA?CC:VV` - raw ROM patch with a compare value.

/// A decoded cheat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cheat {
    /// Substitute `value` for CPU reads of `addr` ($8000-$FFFF), only when
    /// the cartridge byte equals `compare` if one is given.
    Rom {
        addr: u16,
        value: u8,
        compare: Option<u8>,
    },
    /// Force `value` into RAM at `addr` once per frame.
    Ram { addr: u16, value: u8 },
}

/// Game Genie letter -> nibble.
const GG_LETTERS: &[u8; 16] = b"APZLGITYEOXUKSVN";

/// Parse a cheat code in any accepted form.
pub fn parse(code: &str) -> Result<Cheat, String> {
    let code = code.trim().to_ascii_uppercase();
    if code.is_empty() {
        return Err("EMPTY CODE".into());
    }
    if code.contains(':') {
        return parse_raw(&code);
    }
    parse_game_genie(&code)
}

fn parse_game_genie(code: &str) -> Result<Cheat, String> {
    if code.len() != 6 && code.len() != 8 {
        return Err("GAME GENIE CODES ARE 6 OR 8 LETTERS".into());
    }
    let mut n = [0u16; 8];
    for (i, c) in code.bytes().enumerate() {
        n[i] = GG_LETTERS
            .iter()
            .position(|&l| l == c)
            .ok_or_else(|| format!("BAD LETTER {}", c as char))? as u16;
    }
    let addr = 0x8000
        | ((n[3] & 7) << 12)
        | ((n[5] & 7) << 8)
        | ((n[4] & 8) << 8)
        | ((n[2] & 7) << 4)
        | ((n[1] & 8) << 4)
        | (n[4] & 7)
        | (n[3] & 8);
    let low = ((n[1] & 7) << 4) | ((n[0] & 8) << 4) | (n[0] & 7);
    if code.len() == 6 {
        Ok(Cheat::Rom {
            addr,
            value: (low | (n[5] & 8)) as u8,
            compare: None,
        })
    } else {
        Ok(Cheat::Rom {
            addr,
            value: (low | (n[7] & 8)) as u8,
            compare: Some((((n[7] & 7) << 4) | ((n[6] & 8) << 4) | (n[6] & 7) | (n[5] & 8)) as u8),
        })
    }
}

fn hex<T: TryFrom<u32>>(s: &str, digits: usize) -> Result<T, String> {
    if s.is_empty() || s.len() > digits || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("BAD HEX {s}"));
    }
    let v = u32::from_str_radix(s, 16).map_err(|_| format!("BAD HEX {s}"))?;
    T::try_from(v).map_err(|_| format!("BAD HEX {s}"))
}

fn parse_raw(code: &str) -> Result<Cheat, String> {
    let (lhs, value) = code.split_once(':').ok_or("MISSING :")?;
    let value: u8 = hex(value, 2)?;
    let (addr, compare) = match lhs.split_once('?') {
        Some((a, c)) => (hex::<u16>(a, 4)?, Some(hex::<u8>(c, 2)?)),
        None => (hex::<u16>(lhs, 4)?, None),
    };
    match addr {
        0x8000..=0xFFFF => Ok(Cheat::Rom {
            addr,
            value,
            compare,
        }),
        _ if compare.is_some() => Err("COMPARE ONLY WORKS ON ROM".into()),
        0x0000..=0x1FFF | 0x6000..=0x7FFF => Ok(Cheat::Ram { addr, value }),
        _ => Err("ADDRESS NOT RAM OR ROM".into()),
    }
}

/// A ROM read patch installed on the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomPatch {
    pub addr: u16,
    pub value: u8,
    pub compare: Option<u8>,
}

impl RomPatch {
    /// Apply this patch to a cartridge read of `addr` that returned `orig`.
    #[inline]
    pub fn apply(&self, addr: u16, orig: u8) -> Option<u8> {
        (addr == self.addr && self.compare.is_none_or(|c| c == orig)).then_some(self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_letter_game_genie() {
        // Super Mario Bros. infinite lives.
        assert_eq!(
            parse("sxiopo").unwrap(),
            Cheat::Rom {
                addr: 0x91D9,
                value: 0xAD,
                compare: None
            }
        );
    }

    #[test]
    fn eight_letter_game_genie() {
        // Round trip against the encoder below for a few values.
        for (addr, value, cmp) in [
            (0x8000u16, 0x00u8, 0xFFu8),
            (0xD1E2, 0x5A, 0xA5),
            (0xFFFF, 0x80, 0x01),
        ] {
            let code = encode8(addr, value, cmp);
            assert_eq!(
                parse(&code).unwrap(),
                Cheat::Rom {
                    addr,
                    value,
                    compare: Some(cmp)
                },
                "{code}"
            );
        }
    }

    /// Reference Game Genie encoder (inverse of the decoder's bit shuffle).
    fn encode8(addr: u16, value: u8, cmp: u8) -> String {
        let (a, v, c) = (addr as u32, value as u32, cmp as u32);
        let n = [
            (v & 7) | ((v >> 4) & 8),
            ((v >> 4) & 7) | ((a >> 4) & 8),
            (a >> 4) & 7,
            ((a >> 12) & 7) | (a & 8),
            (a & 7) | ((a >> 8) & 8),
            ((a >> 8) & 7) | (c & 8),
            (c & 7) | ((c >> 4) & 8),
            ((c >> 4) & 7) | (v & 8),
        ];
        n.iter().map(|&x| GG_LETTERS[x as usize] as char).collect()
    }

    #[test]
    fn raw_forms() {
        assert_eq!(
            parse("075a:09").unwrap(),
            Cheat::Ram {
                addr: 0x075A,
                value: 0x09
            }
        );
        assert_eq!(
            parse("C123?10:EA").unwrap(),
            Cheat::Rom {
                addr: 0xC123,
                value: 0xEA,
                compare: Some(0x10)
            }
        );
        assert!(parse("4016:01").is_err());
        assert!(parse("0300?01:02").is_err());
        assert!(parse("12345:00").is_err());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("").is_err());
        assert!(parse("ABCDE").is_err());
        assert!(parse("QQQQQQ").is_err());
    }

    #[test]
    fn patch_compare() {
        let p = RomPatch {
            addr: 0x8000,
            value: 1,
            compare: Some(7),
        };
        assert_eq!(p.apply(0x8000, 7), Some(1));
        assert_eq!(p.apply(0x8000, 6), None);
        assert_eq!(p.apply(0x8001, 7), None);
    }
}
