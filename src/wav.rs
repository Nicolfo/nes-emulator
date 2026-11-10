//! Minimal RIFF/WAVE reader for host-supplied audio (the speech samples of
//! boards whose sound chip keeps its recordings in an undumped mask ROM).
//!
//! Accepts uncompressed PCM - 8-bit unsigned or 16-bit signed, any channel
//! count (downmixed to mono), any sample rate - including the
//! WAVE_FORMAT_EXTENSIBLE wrapper around PCM. Input is untrusted: every
//! length is bounds-checked and oversized files are rejected rather than
//! allocated.

/// Longest accepted clip, in sample frames (~6 minutes at 44.1 kHz).
pub const MAX_FRAMES: usize = 16 * 1024 * 1024;

const FORMAT_PCM: u16 = 1;
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

fn u16_at(d: &[u8], off: usize) -> Option<u16> {
    d.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], off: usize) -> Option<u32> {
    d.get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Decode a WAV file into mono samples in -1.0..=1.0 and its sample rate.
pub fn parse(data: &[u8]) -> Result<(Vec<f32>, u32), String> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // (tag, channels, rate, bits)
    let mut pcm: Option<&[u8]> = None;
    let mut off = 12usize;
    while off + 8 <= data.len() {
        let id = &data[off..off + 4];
        let len = u32_at(data, off + 4).ok_or("truncated chunk header")? as usize;
        let body_start = off + 8;
        // A truncated final chunk keeps what is there (common for data).
        let body_end = body_start.saturating_add(len).min(data.len());
        let body = &data[body_start..body_end];
        match id {
            b"fmt " => {
                let tag = u16_at(body, 0).ok_or("short fmt chunk")?;
                let channels = u16_at(body, 2).ok_or("short fmt chunk")?;
                let rate = u32_at(body, 4).ok_or("short fmt chunk")?;
                let bits = u16_at(body, 14).ok_or("short fmt chunk")?;
                let tag = if tag == FORMAT_EXTENSIBLE {
                    // The real format tag is the first word of the GUID.
                    u16_at(body, 24).ok_or("short extensible fmt chunk")?
                } else {
                    tag
                };
                fmt = Some((tag, channels, rate, bits));
            }
            b"data" => pcm = Some(body),
            _ => {}
        }
        // Chunks are word-aligned.
        off = body_start.saturating_add(len).saturating_add(len & 1);
    }
    let (tag, channels, rate, bits) = fmt.ok_or("missing fmt chunk")?;
    let pcm = pcm.ok_or("missing data chunk")?;
    if tag != FORMAT_PCM {
        return Err(format!("unsupported WAV format {tag} (PCM only)"));
    }
    if channels == 0 || channels > 8 {
        return Err(format!("unsupported channel count {channels}"));
    }
    if rate == 0 || rate > 384_000 {
        return Err(format!("unsupported sample rate {rate}"));
    }
    let bytes_per_sample = match bits {
        8 => 1,
        16 => 2,
        _ => return Err(format!("unsupported bit depth {bits} (8 or 16)")),
    };
    let frame = bytes_per_sample * channels as usize;
    let frames = pcm.len() / frame;
    if frames > MAX_FRAMES {
        return Err("WAV file too long".into());
    }
    let mut out = Vec::with_capacity(frames);
    for f in pcm.chunks_exact(frame) {
        let sum: f32 = f
            .chunks_exact(bytes_per_sample)
            .map(|s| match bytes_per_sample {
                1 => (s[0] as f32 - 128.0) / 128.0,
                _ => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
            })
            .sum();
        out.push(sum / channels as f32);
    }
    Ok((out, rate))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a WAV file from raw PCM bytes.
    pub(crate) fn wav(channels: u16, rate: u32, bits: u16, pcm: &[u8]) -> Vec<u8> {
        let mut d = b"RIFF\0\0\0\0WAVE".to_vec();
        d.extend_from_slice(b"fmt ");
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&FORMAT_PCM.to_le_bytes());
        d.extend_from_slice(&channels.to_le_bytes());
        d.extend_from_slice(&rate.to_le_bytes());
        let block = channels as u32 * bits as u32 / 8;
        d.extend_from_slice(&(rate * block).to_le_bytes());
        d.extend_from_slice(&(block as u16).to_le_bytes());
        d.extend_from_slice(&bits.to_le_bytes());
        // An unknown chunk with an odd length (padded) before the data.
        d.extend_from_slice(b"LIST");
        d.extend_from_slice(&3u32.to_le_bytes());
        d.extend_from_slice(&[1, 2, 3, 0]);
        d.extend_from_slice(b"data");
        d.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        d.extend_from_slice(pcm);
        let riff_len = (d.len() - 8) as u32;
        d[4..8].copy_from_slice(&riff_len.to_le_bytes());
        d
    }

    #[test]
    fn mono_8bit_unsigned() {
        let (s, rate) = parse(&wav(1, 8000, 8, &[128, 255, 0])).unwrap();
        assert_eq!(rate, 8000);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], 0.0);
        assert!(s[1] > 0.99 && s[2] == -1.0);
    }

    #[test]
    fn stereo_16bit_downmixes() {
        let mut pcm = Vec::new();
        for (l, r) in [(16384i16, -16384i16), (32767, 32767)] {
            pcm.extend_from_slice(&l.to_le_bytes());
            pcm.extend_from_slice(&r.to_le_bytes());
        }
        let (s, rate) = parse(&wav(2, 44100, 16, &pcm)).unwrap();
        assert_eq!(rate, 44100);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], 0.0);
        assert!(s[1] > 0.99);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(parse(b"").is_err());
        assert!(parse(b"RIFF\0\0\0\0WAVX").is_err());
        assert!(parse(&wav(1, 8000, 24, &[0; 6])).is_err(), "24-bit");
        assert!(parse(&wav(0, 8000, 8, &[0; 6])).is_err(), "no channels");
        assert!(parse(&wav(1, 0, 8, &[0; 6])).is_err(), "zero rate");
        // fmt only, no data.
        let f = wav(1, 8000, 8, &[]);
        let no_data = &f[..f.len() - 8];
        assert!(parse(no_data).is_err());
    }

    #[test]
    fn truncated_data_keeps_what_is_there() {
        let mut f = wav(1, 8000, 16, &[0, 0x40, 0, 0x40, 0, 0x40]);
        f.truncate(f.len() - 3); // cut mid-frame
        let (s, _) = parse(&f).unwrap();
        assert_eq!(s.len(), 1);
        // Any prefix of a valid file parses or errors, never panics.
        let full = wav(2, 22050, 16, &[1; 40]);
        for n in 0..full.len() {
            let _ = parse(&full[..n]);
        }
    }

    #[test]
    fn huge_declared_lengths_do_not_panic() {
        let mut f = wav(1, 8000, 8, &[1, 2, 3]);
        // Claim a 4GB data chunk.
        let pos = f.windows(4).position(|w| w == b"data").unwrap();
        f[pos + 4..pos + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        let (s, _) = parse(&f).unwrap();
        assert_eq!(s.len(), 3);
    }
}
