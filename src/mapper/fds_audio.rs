//! Famicom Disk System expansion sound: one 64-step, 6-bit wavetable channel
//! with a volume envelope, frequency-modulated by a second unit that walks a
//! 32-entry (stored twice, 64-position) table of 3-bit pitch deltas.
//!
//! Self-contained so both the FDS mapper and an NSF player can own one. The
//! output passes through the RAAM's ~2 kHz RC low-pass, applied per CPU cycle.

use serde::{Deserialize, Serialize};

/// Master volume ($4089 bits 0-1) as a multiplier over 2/2, 2/3, 2/4, 2/5;
/// scaled so `wave(63) * gain(32) * 36 / 1152 = 63` at full volume.
const MASTER_VOLUME: [u32; 4] = [36, 24, 17, 14];

/// Modulation table entry -> counter delta; 4 resets the counter to 0.
const MOD_DELTA: [i32; 8] = [0, 1, 2, 4, 0, -4, -2, -1];

/// One-pole low-pass at ~2 kHz, clocked at the NTSC CPU rate
/// (1 - exp(-2*pi*2000/1789773)).
const LOWPASS_ALPHA: f32 = 0.00700;

/// FDS full-scale output in APU units: about 2.4x one 2A03 square at
/// maximum volume (0.149), per the nesdev mixing notes.
const FULL_SCALE: f32 = 0.36;

/// Volume or modulation envelope ($4080 / $4084).
#[derive(Serialize, Deserialize, Clone, Default)]
struct Envelope {
    speed: u8,
    gain: u8,
    increase: bool,
    disabled: bool,
    timer: u32,
}

impl Envelope {
    fn write(&mut self, val: u8, master_speed: u8) {
        self.speed = val & 0x3F;
        self.increase = val & 0x40 != 0;
        self.disabled = val & 0x80 != 0;
        if self.disabled {
            self.gain = val & 0x3F;
        }
        self.reset_timer(master_speed);
    }

    fn reset_timer(&mut self, master_speed: u8) {
        self.timer = 8 * (self.speed as u32 + 1) * master_speed as u32;
    }

    /// Returns true when the gain stepped.
    fn tick(&mut self, master_speed: u8) -> bool {
        if self.disabled || master_speed == 0 {
            return false;
        }
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            self.reset_timer(master_speed);
            if self.increase && self.gain < 32 {
                self.gain += 1;
            } else if !self.increase && self.gain > 0 {
                self.gain -= 1;
            }
            return true;
        }
        false
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct FdsAudio {
    #[serde(with = "crate::savestate::byte_array")]
    wave: [u8; 64],
    wave_write: bool,
    wave_halt: bool,
    wave_freq: u16,
    wave_acc: u32,
    wave_pos: u8,
    env_halt: bool,
    master_volume: u8,
    master_env_speed: u8,
    volume: Envelope,

    #[serde(with = "crate::savestate::byte_array")]
    mod_table: [u8; 64],
    mod_pos: u8,
    mod_halt: bool,
    mod_freq: u16,
    mod_acc: u32,
    /// 7-bit signed sweep bias counter.
    mod_counter: i8,
    modulation: Envelope,

    /// Latest unfiltered DAC level (0..=63) and the low-passed value.
    level: u8,
    filtered: f32,
}

impl Default for FdsAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl FdsAudio {
    pub(crate) fn new() -> Self {
        FdsAudio {
            wave: [0; 64],
            wave_write: false,
            wave_halt: true,
            wave_freq: 0,
            wave_acc: 0,
            wave_pos: 0,
            env_halt: false,
            master_volume: 0,
            master_env_speed: 0xE8,
            volume: Envelope::default(),
            mod_table: [0; 64],
            mod_pos: 0,
            mod_halt: true,
            mod_freq: 0,
            mod_acc: 0,
            mod_counter: 0,
            modulation: Envelope::default(),
            level: 0,
            filtered: 0.0,
        }
    }

    /// Register write in $4040-$408A (other addresses are ignored).
    pub(crate) fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x4040..=0x407F => {
                if self.wave_write {
                    self.wave[(addr & 0x3F) as usize] = val & 0x3F;
                }
            }
            0x4080 => self.volume.write(val, self.master_env_speed),
            0x4082 => self.wave_freq = (self.wave_freq & 0x0F00) | val as u16,
            0x4083 => {
                self.wave_freq = (self.wave_freq & 0x00FF) | ((val as u16 & 0x0F) << 8);
                self.env_halt = val & 0x40 != 0;
                self.wave_halt = val & 0x80 != 0;
                if self.wave_halt {
                    self.wave_pos = 0;
                    self.wave_acc = 0;
                }
                if self.env_halt {
                    self.volume.reset_timer(self.master_env_speed);
                    self.modulation.reset_timer(self.master_env_speed);
                }
            }
            0x4084 => self.modulation.write(val, self.master_env_speed),
            0x4085 => {
                // 7-bit two's complement.
                self.mod_counter = ((val << 1) as i8) >> 1;
            }
            0x4086 => self.mod_freq = (self.mod_freq & 0x0F00) | val as u16,
            0x4087 => {
                self.mod_freq = (self.mod_freq & 0x00FF) | ((val as u16 & 0x0F) << 8);
                self.mod_halt = val & 0x80 != 0;
                if self.mod_halt {
                    self.mod_acc = 0;
                }
            }
            0x4088 => {
                // The table only accepts writes while the unit is halted; each
                // write fills two consecutive positions.
                if self.mod_halt {
                    let p = self.mod_pos as usize;
                    self.mod_table[p] = val & 7;
                    self.mod_table[(p + 1) & 0x3F] = val & 7;
                    self.mod_pos = (self.mod_pos + 2) & 0x3F;
                }
            }
            0x4089 => {
                self.wave_write = val & 0x80 != 0;
                self.master_volume = val & 3;
            }
            0x408A => self.master_env_speed = val,
            _ => {}
        }
    }

    /// Readable registers: wavetable RAM ($4040-$407F), volume gain ($4090)
    /// and modulation gain ($4092). Bits 6-7 are open bus on hardware; they
    /// read back as $40 here (the usual open-bus value after `LDA $40xx`).
    pub(crate) fn read(&self, addr: u16) -> Option<u8> {
        match addr {
            0x4040..=0x407F => Some(self.wave[(addr & 0x3F) as usize] | 0x40),
            0x4090 => Some(self.volume.gain | 0x40),
            0x4092 => Some(self.modulation.gain | 0x40),
            _ => None,
        }
    }

    /// Pitch offset the modulator applies to the wave frequency right now
    /// (the nesdev reference computation, including its rounding quirks).
    fn mod_output(&self) -> i32 {
        if self.mod_halt {
            return 0;
        }
        let mut temp = self.mod_counter as i32 * self.modulation.gain as i32;
        let remainder = temp & 0x0F;
        temp >>= 4;
        if remainder > 0 && temp & 0x80 == 0 {
            if self.mod_counter < 0 {
                temp -= 1;
            } else {
                temp += 2;
            }
        }
        if temp >= 192 {
            temp -= 256;
        } else if temp < -64 {
            temp += 256;
        }
        temp *= self.wave_freq as i32;
        let remainder = temp & 0x3F;
        temp >>= 6;
        if remainder >= 32 {
            temp += 1;
        }
        temp
    }

    fn step_mod_counter(&mut self, entry: u8) {
        if entry == 4 {
            self.mod_counter = 0;
            return;
        }
        let mut c = self.mod_counter as i32 + MOD_DELTA[(entry & 7) as usize];
        if c >= 64 {
            c -= 128;
        } else if c < -64 {
            c += 128;
        }
        self.mod_counter = c as i8;
    }

    /// Advance one CPU cycle.
    pub(crate) fn clock(&mut self) {
        if !self.wave_halt && !self.env_halt {
            self.volume.tick(self.master_env_speed);
            self.modulation.tick(self.master_env_speed);
        }

        if !self.mod_halt && self.mod_freq > 0 {
            self.mod_acc += self.mod_freq as u32;
            if self.mod_acc > 0xFFFF {
                self.mod_acc &= 0xFFFF;
                let entry = self.mod_table[(self.mod_pos & 0x3F) as usize];
                self.step_mod_counter(entry);
                self.mod_pos = (self.mod_pos + 1) & 0x3F;
            }
        }

        if !self.wave_halt && !self.wave_write {
            let pitch = self.wave_freq as i32 + self.mod_output();
            if pitch > 0 {
                self.wave_acc += pitch as u32;
                if self.wave_acc > 0xFFFF {
                    self.wave_acc &= 0xFFFF;
                    self.wave_pos = (self.wave_pos + 1) & 0x3F;
                }
            }
        }

        // While wavetable writes are enabled the DAC holds its last level.
        if !self.wave_write {
            let gain = self.volume.gain.min(32) as u32;
            let level = gain * MASTER_VOLUME[(self.master_volume & 3) as usize];
            self.level = ((self.wave[(self.wave_pos & 0x3F) as usize] as u32 * level) / 1152) as u8;
        }
        self.filtered += (self.level as f32 - self.filtered) * LOWPASS_ALPHA;
    }

    /// Current output in APU units.
    pub(crate) fn sample(&self) -> f32 {
        self.filtered / 63.0 * FULL_SCALE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Square-ish wave at full volume with envelopes off.
    fn playing() -> FdsAudio {
        let mut a = FdsAudio::new();
        a.write(0x4089, 0x80); // wave write enable, full master volume
        for i in 0..64u16 {
            a.write(0x4040 + i, if i < 32 { 63 } else { 0 });
        }
        a.write(0x4089, 0x00);
        a.write(0x4080, 0x80 | 32); // envelope off, gain 32
        a.write(0x4082, 0x00);
        a.write(0x4083, 0x04); // freq $400, running
        a
    }

    #[test]
    fn wavetable_produces_output() {
        let mut a = playing();
        let mut peak = 0.0f32;
        let mut trough = f32::MAX;
        for _ in 0..200_000 {
            a.clock();
            peak = peak.max(a.sample());
            trough = trough.min(a.sample());
        }
        assert!(peak > 0.2, "expected audible FDS output, got {peak}");
        assert!(peak - trough > 0.1, "output should oscillate");
    }

    #[test]
    fn halted_wave_is_silent() {
        let mut a = playing();
        a.write(0x4083, 0x80); // halt: position resets to 0
        a.write(0x4040, 0);
        for _ in 0..50_000 {
            a.clock();
        }
        // Position 0 holds 63 from `playing`; write-protected while not
        // enabled, so the halted output is a constant DC level.
        let s1 = a.sample();
        a.clock();
        assert!((a.sample() - s1).abs() < 1e-3);
    }

    #[test]
    fn mod_table_writes_only_while_halted() {
        let mut a = FdsAudio::new();
        a.write(0x4087, 0x80);
        a.write(0x4088, 3);
        assert_eq!(&a.mod_table[..3], &[3, 3, 0]);
        a.write(0x4087, 0x00);
        a.write(0x4088, 5);
        assert_eq!(a.mod_table[2], 0);
    }

    #[test]
    fn mod_counter_is_seven_bit_signed() {
        let mut a = FdsAudio::new();
        a.write(0x4085, 0x7F);
        assert_eq!(a.mod_counter, -1);
        a.write(0x4085, 0x3F);
        assert_eq!(a.mod_counter, 63);
        a.step_mod_counter(3); // +4 wraps past 63
        assert_eq!(a.mod_counter, -61);
    }

    #[test]
    fn gain_registers_read_back() {
        let mut a = FdsAudio::new();
        a.write(0x4080, 0x80 | 20);
        a.write(0x4084, 0x80 | 7);
        assert_eq!(a.read(0x4090), Some(0x40 | 20));
        assert_eq!(a.read(0x4092), Some(0x40 | 7));
        assert_eq!(a.read(0x4091), None);
    }
}
