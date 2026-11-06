use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// FME-7 / Sunsoft 5B (mapper 69, Gimmick!, Batman: Return of the Joker):
/// command/parameter banking, a 16-bit CPU-cycle IRQ counter, and (on the
/// 5B) a YM2149-derived sound generator: three tone channels, the 17-bit
/// noise generator, the 32-step envelope with all 16 shapes, and the
/// logarithmic volume curve (3 dB per volume step, 1.5 dB per envelope step).
#[derive(Serialize, Deserialize)]
pub struct Fme7 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    mirroring: Mirroring,
    command: u8,
    chr_banks: [u8; 8],
    prg_banks: [u8; 3],
    // Command 8: bits 0-5 bank, bit 6 RAM (vs ROM), bit 7 RAM enable.
    wram_ctrl: u8,
    irq_enabled: bool,
    irq_counter_enabled: bool,
    irq_counter: u16,
    irq_line: bool,
    audio: Sunsoft5b,
}

impl Fme7 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Fme7 {
            prg,
            chr,
            chr_is_ram,
            prg_ram: vec![0; 0x2000],
            mirroring,
            command: 0,
            chr_banks: [0; 8],
            prg_banks: [0; 3],
            wram_ctrl: 0,
            irq_enabled: false,
            irq_counter_enabled: false,
            irq_counter: 0,
            irq_line: false,
            audio: Sunsoft5b::new(),
        }
    }

    fn prg_offset(&self, addr: u16) -> usize {
        let banks = self.prg.len() / 0x2000;
        let bank = match addr {
            0x8000..=0x9FFF => self.prg_banks[0] as usize % banks,
            0xA000..=0xBFFF => self.prg_banks[1] as usize % banks,
            0xC000..=0xDFFF => self.prg_banks[2] as usize % banks,
            _ => banks - 1,
        };
        bank * 0x2000 + (addr as usize & 0x1FFF)
    }

    fn run_command(&mut self, val: u8) {
        match self.command {
            0x0..=0x7 => self.chr_banks[self.command as usize] = val,
            0x8 => self.wram_ctrl = val,
            0x9..=0xB => self.prg_banks[self.command as usize - 9] = val & 0x3F,
            0xC => {
                if self.mirroring != Mirroring::FourScreen {
                    self.mirroring = match val & 3 {
                        0 => Mirroring::Vertical,
                        1 => Mirroring::Horizontal,
                        2 => Mirroring::SingleScreenLo,
                        _ => Mirroring::SingleScreenHi,
                    };
                }
            }
            0xD => {
                // Any write acknowledges a pending IRQ.
                self.irq_line = false;
                self.irq_enabled = val & 0x01 != 0;
                self.irq_counter_enabled = val & 0x80 != 0;
            }
            0xE => self.irq_counter = (self.irq_counter & 0xFF00) | val as u16,
            _ => self.irq_counter = (self.irq_counter & 0x00FF) | ((val as u16) << 8),
        }
    }
}

impl Mapper for Fme7 {
    crate::impl_mapper_savestate!(chr, prg_ram);

    fn set_ram_sizes(&mut self, prg_ram: usize, chr_ram: usize) {
        if prg_ram > 0 {
            self.prg_ram = vec![0; prg_ram];
        }
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }
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
                // Writes only land when RAM is selected and enabled.
                if self.wram_ctrl & 0xC0 == 0xC0 {
                    self.prg_ram[(addr & 0x1FFF) as usize] = val;
                }
            }
            0x8000..=0x9FFF => self.command = val & 0x0F,
            0xA000..=0xBFFF => self.run_command(val),
            0xC000..=0xDFFF => self.audio.reg_select = val & 0x0F,
            0xE000..=0xFFFF => self.audio.write_reg(val),
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let banks = self.chr.len() / 0x400;
        let bank = self.chr_banks[(addr >> 10) as usize & 7] as usize % banks;
        self.chr[bank * 0x400 + (addr as usize & 0x3FF)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        if self.chr_is_ram {
            let banks = self.chr.len() / 0x400;
            let bank = self.chr_banks[(addr >> 10) as usize & 7] as usize % banks;
            self.chr[bank * 0x400 + (addr as usize & 0x3FF)] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        if self.wram_ctrl & 0x40 == 0 {
            // ROM selected at $6000-$7FFF.
            let banks = self.prg.len() / 0x2000;
            let bank = (self.wram_ctrl & 0x3F) as usize % banks;
            Some(self.prg[bank * 0x2000 + (addr as usize & 0x1FFF)])
        } else if self.wram_ctrl & 0x80 != 0 {
            Some(self.prg_ram[(addr & 0x1FFF) as usize])
        } else {
            None // RAM selected but disabled: open bus
        }
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.prg_ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.prg_ram)
    }

    fn irq(&self) -> bool {
        self.irq_line
    }

    fn cpu_clock(&mut self) {
        if self.irq_counter_enabled {
            let (next, wrapped) = self.irq_counter.overflowing_sub(1);
            self.irq_counter = next;
            if wrapped && self.irq_enabled {
                self.irq_line = true;
            }
        }
        self.audio.clock();
    }

    fn audio_sample(&self) -> f32 {
        self.audio.sample()
    }
}

/// Sunsoft 5B sound: three YM2149 square-tone channels. The chip divides
/// the CPU clock by 16 per tone step (twice the YM2149's /8, at twice the
/// typical clock - same pitch).
#[derive(Serialize, Deserialize)]
pub(crate) struct Sunsoft5b {
    pub(crate) reg_select: u8,
    regs: [u8; 16],
    prescaler: u8,
    counters: [u16; 3],
    output: [bool; 3],
    /// Noise: period counter, /2 phase and the 17-bit LFSR.
    #[serde(default)]
    noise_counter: u8,
    #[serde(default)]
    noise_phase: bool,
    #[serde(default = "noise_seed")]
    lfsr: u32,
    /// Envelope: period counter, 5-bit step (31 down to 0), direction and
    /// whether the shape has finished and holds its level.
    #[serde(default)]
    env_counter: u16,
    #[serde(default)]
    env_step: u8,
    #[serde(default)]
    env_attack: bool,
    #[serde(default)]
    env_holding: bool,
}

fn noise_seed() -> u32 {
    1
}

/// Output amplitude per 5-bit level: 1.5 dB per step, silent at 0, scaled
/// so full volume matches the APU-relative level the mixer expects.
fn level_amplitude(level: u8) -> f32 {
    if level == 0 {
        0.0
    } else {
        0.15 * 10f32.powf((level as f32 - 31.0) * 1.5 / 20.0)
    }
}

impl Sunsoft5b {
    pub(crate) fn new() -> Self {
        Sunsoft5b {
            reg_select: 0,
            regs: [0; 16],
            // Tones start disabled (mixer bits set = disabled).
            prescaler: 0,
            counters: [0; 3],
            output: [false; 3],
            noise_counter: 0,
            noise_phase: false,
            lfsr: noise_seed(),
            env_counter: 0,
            env_step: 0,
            env_attack: false,
            env_holding: true,
        }
    }

    pub(crate) fn write_reg(&mut self, val: u8) {
        let r = (self.reg_select & 0x0F) as usize;
        self.regs[r] = val;
        if r == 13 {
            // Writing the shape restarts the envelope.
            self.env_step = 31;
            self.env_attack = val & 0x04 != 0;
            self.env_holding = false;
            self.env_counter = 0;
        }
    }

    fn period(&self, ch: usize) -> u16 {
        let p = (self.regs[ch * 2] as u16) | ((self.regs[ch * 2 + 1] as u16 & 0x0F) << 8);
        p.max(1)
    }

    fn noise_period(&self) -> u8 {
        (self.regs[6] & 0x1F).max(1)
    }

    fn env_period(&self) -> u16 {
        (self.regs[11] as u16 | (self.regs[12] as u16) << 8).max(1)
    }

    /// Current envelope level, 0-31.
    fn env_level(&self) -> u8 {
        if self.env_attack {
            31 - self.env_step
        } else {
            self.env_step
        }
    }

    fn clock_envelope(&mut self) {
        if self.env_holding {
            return;
        }
        if self.env_step > 0 {
            self.env_step -= 1;
            return;
        }
        // End of a ramp: shape bits C(3) A(2) Alt(1) Hold(0).
        let shape = self.regs[13];
        if shape & 0x08 == 0 {
            // No continue: drop to 0 and stay there.
            self.env_attack = false;
            self.env_holding = true;
        } else if shape & 0x01 != 0 {
            // Hold: keep the final level, or its opposite when alternating.
            if shape & 0x02 != 0 {
                self.env_attack = !self.env_attack;
            }
            self.env_attack = !self.env_attack;
            self.env_step = 31;
            self.env_holding = true;
            // Holding at step 31 of the flipped ramp shows the final level.
        } else {
            if shape & 0x02 != 0 {
                self.env_attack = !self.env_attack;
            }
            self.env_step = 31;
        }
    }

    pub(crate) fn clock(&mut self) {
        self.prescaler = (self.prescaler + 1) & 0x0F;
        if self.prescaler != 0 {
            return;
        }
        for ch in 0..3 {
            if self.counters[ch] == 0 {
                self.counters[ch] = self.period(ch);
            }
            self.counters[ch] -= 1;
            if self.counters[ch] == 0 {
                self.output[ch] = !self.output[ch];
            }
        }
        // Noise steps at half the rate of a tone with the same period.
        if self.noise_counter == 0 {
            self.noise_counter = self.noise_period();
        }
        self.noise_counter -= 1;
        if self.noise_counter == 0 {
            self.noise_phase = !self.noise_phase;
            if self.noise_phase {
                let bit = (self.lfsr ^ (self.lfsr >> 3)) & 1;
                self.lfsr = (self.lfsr >> 1) | (bit << 16);
            }
        }
        if self.env_counter == 0 {
            self.env_counter = self.env_period();
        }
        self.env_counter -= 1;
        if self.env_counter == 0 {
            self.clock_envelope();
        }
    }

    pub(crate) fn sample(&self) -> f32 {
        let noise = self.lfsr & 1 != 0;
        let mut s = 0.0;
        for ch in 0..3 {
            // Mixer reg 7: bits 0-2 disable tone, bits 3-5 disable noise. A
            // channel with both disabled outputs a constant level (used for
            // volume-register PCM).
            let tone_off = self.regs[7] & (1 << ch) != 0;
            let noise_off = self.regs[7] & (8 << ch) != 0;
            if !((tone_off || self.output[ch]) && (noise_off || noise)) {
                continue;
            }
            let vol = self.regs[8 + ch];
            let level = if vol & 0x10 != 0 {
                self.env_level()
            } else if vol & 0x0F == 0 {
                0
            } else {
                // 4-bit volume = 3 dB steps = every other envelope level.
                (vol & 0x0F) * 2 + 1
            };
            s += level_amplitude(level);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fme7() -> Fme7 {
        // 8 PRG banks (64KB), 16 CHR 1KB banks; byte = bank index.
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Fme7::new(prg, chr, Mirroring::Vertical)
    }

    fn cmd(m: &mut Fme7, command: u8, val: u8) {
        m.cpu_write(0x8000, command);
        m.cpu_write(0xA000, val);
    }

    #[test]
    fn prg_banking() {
        let mut m = fme7();
        cmd(&mut m, 0x9, 3);
        cmd(&mut m, 0xA, 4);
        cmd(&mut m, 0xB, 5);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xA000), 4);
        assert_eq!(m.cpu_read(0xC000), 5);
        assert_eq!(m.cpu_read(0xE000), 7); // fixed last
    }

    #[test]
    fn chr_banking() {
        let mut m = fme7();
        cmd(&mut m, 0x0, 9);
        cmd(&mut m, 0x7, 2);
        assert_eq!(m.ppu_read(0x0000), 9);
        assert_eq!(m.ppu_read(0x1C00), 2);
    }

    #[test]
    fn wram_states() {
        let mut m = fme7();
        // ROM mode: $6000 reads PRG bank from cmd 8 bits 0-5.
        cmd(&mut m, 0x8, 0x02);
        assert_eq!(m.prg_ram_read(0x6000), Some(2));
        // RAM selected but disabled: open bus, writes dropped.
        cmd(&mut m, 0x8, 0x40);
        m.cpu_write(0x6000, 0xAB);
        assert_eq!(m.prg_ram_read(0x6000), None);
        // RAM enabled.
        cmd(&mut m, 0x8, 0xC0);
        m.cpu_write(0x6000, 0xAB);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAB));
    }

    #[test]
    fn mirroring_command() {
        let mut m = fme7();
        cmd(&mut m, 0xC, 1);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        cmd(&mut m, 0xC, 2);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        cmd(&mut m, 0xC, 3);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
    }

    #[test]
    fn irq_counts_cpu_cycles() {
        let mut m = fme7();
        cmd(&mut m, 0xE, 5); // counter low
        cmd(&mut m, 0xF, 0); // counter high
        cmd(&mut m, 0xD, 0x81); // enable counter + IRQ
        // Counter wraps after 6 decrements (5 -> ... -> 0 -> $FFFF).
        for i in 0..6 {
            assert!(!m.irq(), "IRQ too early at cycle {i}");
            m.cpu_clock();
        }
        assert!(m.irq());
        // Writing command D acknowledges.
        cmd(&mut m, 0xD, 0x81);
        assert!(!m.irq());
    }

    #[test]
    fn irq_disabled_counter_does_not_count() {
        let mut m = fme7();
        cmd(&mut m, 0xE, 1);
        cmd(&mut m, 0xF, 0);
        cmd(&mut m, 0xD, 0x01); // IRQ enabled, counter disabled
        for _ in 0..10 {
            m.cpu_clock();
        }
        assert!(!m.irq());
    }

    #[test]
    fn audio_tone_produces_output() {
        let mut m = fme7();
        // Channel A: period 1, volume 15, tone enabled in mixer.
        m.cpu_write(0xC000, 0x00);
        m.cpu_write(0xE000, 0x01); // period A low
        m.cpu_write(0xC000, 0x07);
        m.cpu_write(0xE000, 0xFE); // mixer: only tone A enabled
        m.cpu_write(0xC000, 0x08);
        m.cpu_write(0xE000, 0x0F); // volume A = 15
        let mut peak = 0.0f32;
        for _ in 0..256 {
            m.cpu_clock();
            peak = peak.max(m.audio_sample());
        }
        assert!(peak > 0.1, "expected audible 5B output, got {peak}");
    }

    fn ym(m: &mut Sunsoft5b, r: u8, v: u8) {
        m.reg_select = r;
        m.write_reg(v);
    }

    fn run(m: &mut Sunsoft5b, cycles: usize) -> Vec<f32> {
        (0..cycles)
            .map(|_| {
                m.clock();
                m.sample()
            })
            .collect()
    }

    #[test]
    fn noise_only_channel_is_audible_and_irregular() {
        let mut m = Sunsoft5b::new();
        ym(&mut m, 6, 1); // fastest noise
        ym(&mut m, 7, 0b110_111); // ch A: tone off, noise on
        ym(&mut m, 8, 15);
        let out = run(&mut m, 16 * 400);
        let on = out.iter().filter(|&&v| v > 0.0).count();
        assert!(on > 500 && on < out.len() - 500, "noise toggles: {on}");
    }

    #[test]
    fn envelope_decay_then_hold_at_zero() {
        let mut m = Sunsoft5b::new();
        ym(&mut m, 7, 0b111_111); // A: constant output, level from volume
        ym(&mut m, 8, 0x10); // envelope mode
        ym(&mut m, 11, 1); // fastest envelope
        ym(&mut m, 12, 0);
        ym(&mut m, 13, 0x00); // single decay, then hold 0
        let first = run(&mut m, 16)[15];
        assert!(first > 0.1, "starts loud: {first}");
        let out = run(&mut m, 16 * 40);
        assert_eq!(*out.last().unwrap(), 0.0);
        // Monotonic decay.
        assert!(out.windows(2).all(|w| w[1] <= w[0]));
    }

    #[test]
    fn envelope_sawtooth_repeats_and_attack_hold_stays_high() {
        let mut m = Sunsoft5b::new();
        ym(&mut m, 7, 0b111_111);
        ym(&mut m, 8, 0x10);
        ym(&mut m, 11, 1);
        ym(&mut m, 13, 0x08); // repeating decay (sawtooth)
        let out = run(&mut m, 16 * 100);
        let rises = out.windows(2).filter(|w| w[1] > w[0]).count();
        assert!(rises >= 2, "sawtooth restarts: {rises}");

        ym(&mut m, 13, 0x0D); // attack, then hold at max
        let out = run(&mut m, 16 * 100);
        assert!((out.last().unwrap() - 0.15).abs() < 1e-6);
    }

    #[test]
    fn fixed_volume_steps_are_3db() {
        let mut m = Sunsoft5b::new();
        ym(&mut m, 7, 0b111_111);
        ym(&mut m, 8, 15);
        let hi = run(&mut m, 1)[0];
        ym(&mut m, 8, 14);
        let lo = run(&mut m, 1)[0];
        let db = 20.0 * (hi / lo).log10();
        assert!((db - 3.0).abs() < 0.01, "{db} dB");
    }
}
