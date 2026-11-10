use super::Chr;
use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Jaleco boards with a single bank latch at $6000-$7FFF (no PRG RAM):
///
/// - **86 (JF-13)**: `$6000-$6FFF [.CPP ..CC]`: 32KB PRG bank `PP`, 8KB CHR
///   bank `C..CC` (bit 6 is CHR bit 2). $7000-$7FFF drives the uPD7756
///   speech chip: `[..RS NNNN]` - a write with R=1 and S=0 starts sample
///   `NNNN`, R=0 holds the chip in reset (stops it). The chip's recordings
///   live in its undumped mask ROM, so playback uses host-supplied clips
///   (see [`SpeechPlayer`]). Moero!! Pro Yakyuu, Argus (J).
/// - **87 (J87, also Konami/Taito)**: `$6000-$7FFF [.... ..LH]`: 8KB CHR bank
///   with the two bits swapped (`HL`). PRG is fixed (NROM). City Connection
///   (J), The Goonies (J), Argus, Hyper Olympic.
/// - **140 (JF-11/JF-14)**: `$6000-$7FFF [..PP CCCC]`: 32KB PRG bank and
///   8KB CHR bank. Bio Senshi Dan, Mississippi Satsujin Jiken.
#[derive(Serialize, Deserialize)]
pub struct JalecoLatch {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Chr,
    chr_is_ram: bool,
    mapper_id: u8,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
    #[serde(default)]
    speech: SpeechPlayer,
}

/// NTSC CPU clock, the rate `SpeechPlayer::clock` is called at.
const CPU_HZ: f64 = 1_789_773.0;
/// Speech output level relative to the APU's ~1.0 full scale.
pub(crate) const SPEECH_GAIN: f32 = 0.35;

/// Playback of the NEC uPD7755/uPD7756 ADPCM speech chips on Jaleco boards.
///
/// The chips decode recordings from an internal mask ROM that iNES dumps
/// don't include, so - like other emulators - this plays host-supplied
/// recordings instead: one clip per sample number, set via
/// [`Mapper::set_speech_samples`]. The clips are host data, not machine
/// state: savestates keep only the playback position, and the live clips
/// carry over a restore. Without clips the chip is silent.
#[derive(Serialize, Deserialize, Default)]
pub(crate) struct SpeechPlayer {
    #[serde(skip)]
    samples: Vec<Option<Vec<f32>>>,
    /// Clip samples advanced per CPU cycle.
    #[serde(skip)]
    step: f64,
    /// Sample number playing, if any.
    playing: Option<usize>,
    /// Position in the clip, in clip samples.
    pos: f64,
}

impl SpeechPlayer {
    pub(crate) fn set_samples(&mut self, samples: Vec<Option<Vec<f32>>>, sample_rate: u32) {
        self.samples = samples;
        self.step = sample_rate as f64 / CPU_HZ;
    }

    /// Start sample `index` from the beginning, cutting any current one.
    pub(crate) fn start(&mut self, index: usize) {
        self.playing = Some(index);
        self.pos = 0.0;
    }

    pub(crate) fn stop(&mut self) {
        self.playing = None;
        self.pos = 0.0;
    }

    /// Whether a clip is currently sounding (the chip's BUSY output; no
    /// emulated board wires it to a CPU-readable register).
    #[cfg(test)]
    pub(crate) fn busy(&self) -> bool {
        self.current().is_some()
    }

    fn current(&self) -> Option<&[f32]> {
        let clip = self.samples.get(self.playing?)?.as_deref()?;
        // A hand-edited state may carry a NaN/negative position.
        (self.pos >= 0.0 && (self.pos as usize) < clip.len()).then_some(clip)
    }

    /// Advance one CPU cycle.
    pub(crate) fn clock(&mut self) {
        if self.playing.is_none() {
            return;
        }
        self.pos += self.step;
        if self.current().is_none() {
            self.stop();
        }
    }

    pub(crate) fn sample(&self) -> f32 {
        self.current().map_or(0.0, |clip| {
            clip[self.pos as usize].clamp(-1.0, 1.0) * SPEECH_GAIN
        })
    }

    /// Carry the host-supplied clips over to a state restored from a
    /// savestate (they are not part of it).
    pub(crate) fn adopt_clips_from(&mut self, live: &mut SpeechPlayer) {
        self.samples = std::mem::take(&mut live.samples);
        self.step = live.step;
    }
}

impl JalecoLatch {
    pub fn new(mapper_id: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        JalecoLatch {
            prg,
            chr: chr.into(),
            chr_is_ram,
            mapper_id,
            prg_bank: 0,
            chr_bank: 0,
            mirroring,
            speech: SpeechPlayer::default(),
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x2000;
        (self.chr_bank as usize % banks) * 0x2000 + (addr as usize & 0x1FFF)
    }
}

impl Mapper for JalecoLatch {
    fn save_state(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("serialize mapper state")
    }

    // `impl_mapper_savestate!(chr)`, plus carrying the host-supplied speech
    // clips over (they are not part of the state).
    fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        let mut new: Self = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        super::StateField::restore(&mut new.chr, &mut self.chr, "chr")?;
        new.prg = std::mem::take(&mut self.prg);
        new.speech.adopt_clips_from(&mut self.speech);
        *self = new;
        Ok(())
    }

    fn set_ram_sizes(&mut self, _prg_ram: usize, chr_ram: usize) {
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram].into();
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        // 32KB banking; a 16KB image mirrors into both halves.
        let off = self.prg_bank as usize * 0x8000 + (addr as usize & 0x7FFF);
        self.prg[off % self.prg.len()]
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match (self.mapper_id, addr) {
            (86, 0x6000..=0x6FFF) => {
                self.prg_bank = (val >> 4) & 3;
                self.chr_bank = (val & 3) | ((val >> 4) & 4);
            }
            (86, 0x7000..=0x7FFF) => match val & 0x30 {
                // R (bit 5) high with S (bit 4) low starts sample NNNN.
                0x20 => self.speech.start((val & 0x0F) as usize),
                // R low holds the chip in reset.
                0x00 | 0x10 => self.speech.stop(),
                _ => {}
            },
            (87, 0x6000..=0x7FFF) => {
                self.chr_bank = ((val & 1) << 1) | ((val >> 1) & 1);
            }
            (140, 0x6000..=0x7FFF) => {
                self.prg_bank = (val >> 4) & 3;
                self.chr_bank = val & 0x0F;
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
        self.mirroring
    }

    fn cpu_clock(&mut self) {
        self.speech.clock();
    }

    fn audio_sample(&self) -> f32 {
        self.speech.sample()
    }

    fn has_speech(&self) -> bool {
        self.mapper_id == 86
    }

    fn set_speech_samples(&mut self, samples: Vec<Option<Vec<f32>>>, sample_rate: u32) {
        if self.mapper_id == 86 {
            self.speech.set_samples(samples, sample_rate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(id: u8) -> JalecoLatch {
        let prg: Vec<u8> = (0..4 * 0x8000).map(|i| (i / 0x8000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        JalecoLatch::new(id, prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn jf13_prg_and_split_chr_bits() {
        let mut m = board(86);
        m.cpu_write(0x6000, 0x40 | 0x20 | 0x01); // CHR bit2, PRG 2, CHR 1
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.ppu_read(0x0000), 5);
        m.cpu_write(0x7000, 0xFF); // speech port: no banking effect
        assert_eq!(m.cpu_read(0x8000), 2);
    }

    /// Clip `n` is a constant level of `(n + 1) / 10` lasting `len` samples;
    /// clip 2 is missing.
    pub(crate) fn clips(len: usize) -> Vec<Option<Vec<f32>>> {
        (0..4)
            .map(|n| (n != 2).then(|| vec![(n + 1) as f32 / 10.0; len]))
            .collect()
    }

    #[test]
    fn jf13_speech_trigger_plays_mixes_and_ends() {
        let mut m = board(86);
        assert!(m.has_speech());
        m.cpu_write(0x7000, 0x20 | 1); // start sample 1
        m.cpu_clock();
        assert_eq!(m.audio_sample(), 0.0, "no clips loaded: silent");
        // 8 kHz clip of 80 samples = 10 ms = ~17900 CPU cycles.
        m.set_speech_samples(clips(80), 8000);
        m.cpu_write(0x7000, 0x20 | 1);
        m.cpu_clock();
        assert!((m.audio_sample() - 0.2 * SPEECH_GAIN).abs() < 1e-6);
        assert!(m.speech.busy());
        for _ in 0..18_000 {
            m.cpu_clock();
        }
        assert_eq!(m.audio_sample(), 0.0, "clip finished");
        assert!(!m.speech.busy());
        // A missing clip and an out-of-range number are silent, not panics.
        m.cpu_write(0x7000, 0x20 | 2);
        m.cpu_clock();
        assert_eq!(m.audio_sample(), 0.0);
        m.cpu_write(0x7000, 0x20 | 0x0F);
        m.cpu_clock();
        assert_eq!(m.audio_sample(), 0.0);
    }

    #[test]
    fn jf13_new_start_or_reset_cuts_playback() {
        let mut m = board(86);
        m.set_speech_samples(clips(8000), 8000);
        m.cpu_write(0x7000, 0x20); // sample 0
        m.cpu_clock();
        assert!((m.audio_sample() - 0.1 * SPEECH_GAIN).abs() < 1e-6);
        m.cpu_write(0x7000, 0x20 | 3); // a new start cuts to sample 3
        m.cpu_clock();
        assert!((m.audio_sample() - 0.4 * SPEECH_GAIN).abs() < 1e-6);
        m.cpu_write(0x7000, 0x00); // reset
        m.cpu_clock();
        assert_eq!(m.audio_sample(), 0.0);
    }

    #[test]
    fn jf13_savestate_keeps_position_not_clips() {
        let mut m = board(86);
        m.set_speech_samples(clips(8000), 8000);
        m.cpu_write(0x7000, 0x20 | 1);
        for _ in 0..1000 {
            m.cpu_clock();
        }
        let state = m.save_state();
        let text = String::from_utf8(state.clone()).unwrap();
        assert!(
            !text.contains("samples"),
            "clip data must not be serialized"
        );
        m.cpu_write(0x7000, 0x00);
        m.load_state(&state).unwrap();
        assert!(m.speech.busy(), "playback resumes with the live clips");
        assert!((m.audio_sample() - 0.2 * SPEECH_GAIN).abs() < 1e-6);
    }

    #[test]
    fn only_jf13_has_speech() {
        assert!(!board(87).has_speech());
        assert!(!board(140).has_speech());
        let mut m = board(87);
        m.set_speech_samples(clips(10), 8000);
        m.cpu_write(0x7000, 0x21);
        m.cpu_clock();
        assert_eq!(m.audio_sample(), 0.0);
    }

    #[test]
    fn j87_swaps_chr_bits() {
        let mut m = board(87);
        m.cpu_write(0x6000, 0x01);
        assert_eq!(m.ppu_read(0x0000), 2);
        m.cpu_write(0x7FFF, 0x02);
        assert_eq!(m.ppu_read(0x0000), 1);
        assert_eq!(m.cpu_read(0x8000), 0, "PRG fixed");
    }

    #[test]
    fn jf11_prg_and_chr() {
        let mut m = board(140);
        m.cpu_write(0x6000, 0x37);
        assert_eq!(m.cpu_read(0xFFFF), 3);
        assert_eq!(m.ppu_read(0x1FFF), 7);
    }

    #[test]
    fn sixteen_kb_prg_mirrors() {
        let prg: Vec<u8> = (0..0x4000).map(|i| (i >> 8) as u8).collect();
        let mut m = JalecoLatch::new(87, prg, vec![], Mirroring::Horizontal);
        assert_eq!(m.cpu_read(0x8100), m.cpu_read(0xC100));
        m.ppu_write(0x0010, 0x99); // CHR RAM
        assert_eq!(m.ppu_read(0x0010), 0x99);
    }
}
