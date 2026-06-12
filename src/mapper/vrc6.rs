use super::{Mapper, Mirroring};

/// VRC6 (mapper 24, Akumajou Densetsu): 16KB+8KB PRG banking, 1KB CHR
/// banking, the VRC scanline/cycle IRQ, and expansion audio (two pulse
/// channels with variable duty plus a sawtooth channel).
pub struct Vrc6 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: [u8; 0x2000],
    mirroring: Mirroring,
    prg_16k: u8,
    prg_8k: u8,
    chr_banks: [u8; 8],
    irq: VrcIrq,
    audio: Vrc6Audio,
}

impl Vrc6 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        Vrc6 {
            prg,
            chr,
            prg_ram: [0; 0x2000],
            mirroring,
            prg_16k: 0,
            prg_8k: 0,
            chr_banks: [0; 8],
            irq: VrcIrq::new(),
            audio: Vrc6Audio::new(),
        }
    }
}

impl Mapper for Vrc6 {
    fn cpu_read(&mut self, addr: u16) -> u8 {
        match addr {
            0x8000..=0xBFFF => {
                let banks = self.prg.len() / 0x4000;
                let bank = self.prg_16k as usize % banks;
                self.prg[bank * 0x4000 + (addr as usize & 0x3FFF)]
            }
            0xC000..=0xDFFF => {
                let banks = self.prg.len() / 0x2000;
                let bank = self.prg_8k as usize % banks;
                self.prg[bank * 0x2000 + (addr as usize & 0x1FFF)]
            }
            0xE000..=0xFFFF => {
                let last = self.prg.len() - 0x2000;
                self.prg[last + (addr as usize & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        // VRC6a wires the register lines to A0/A1 directly.
        match (addr & 0xF000, addr & 3) {
            (0x6000, _) if addr < 0x8000 => self.prg_ram[(addr & 0x1FFF) as usize] = val,
            (0x7000, _) => self.prg_ram[(addr & 0x1FFF) as usize] = val,
            (0x8000, _) => self.prg_16k = val & 0x0F,
            (0x9000, 0..=2) => self.audio.pulse_write(0, addr & 3, val),
            (0x9000, _) => self.audio.freq_ctrl = val,
            (0xA000, 0..=2) => self.audio.pulse_write(1, addr & 3, val),
            (0xB000, 0..=2) => self.audio.saw_write(addr & 3, val),
            (0xB000, _) => {
                self.mirroring = match (val >> 2) & 3 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleScreenLo,
                    _ => Mirroring::SingleScreenHi,
                };
                // Bits 0-1 select exotic CHR/NT banking modes; only the
                // common mode 0 (8 x 1KB) is emulated.
            }
            (0xC000, _) => self.prg_8k = val & 0x1F,
            (0xD000, k) => self.chr_banks[k as usize] = val,
            (0xE000, k) => self.chr_banks[4 + k as usize] = val,
            (0xF000, 0) => self.irq.latch = val,
            (0xF000, 1) => self.irq.write_control(val),
            (0xF000, 2) => self.irq.ack(),
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let banks = self.chr.len() / 0x400;
        let bank = self.chr_banks[(addr >> 10) as usize & 7] as usize % banks;
        self.chr[bank * 0x400 + (addr as usize & 0x3FF)]
    }

    fn ppu_write(&mut self, _addr: u16, _val: u8) {
        // CHR is ROM on VRC6 boards.
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        Some(self.prg_ram[(addr & 0x1FFF) as usize])
    }

    fn irq(&self) -> bool {
        self.irq.line
    }

    fn cpu_clock(&mut self) {
        self.irq.clock();
        self.audio.clock();
    }

    fn audio_sample(&self) -> f32 {
        self.audio.sample()
    }
}

/// The shared Konami VRC IRQ: an up-counter from a reloadable latch, in
/// CPU-cycle mode or scanline mode (a 341/3-dot prescaler).
struct VrcIrq {
    latch: u8,
    counter: u8,
    enabled: bool,
    enable_after_ack: bool,
    cycle_mode: bool,
    prescaler: i16,
    line: bool,
}

impl VrcIrq {
    fn new() -> Self {
        VrcIrq {
            latch: 0,
            counter: 0,
            enabled: false,
            enable_after_ack: false,
            cycle_mode: false,
            prescaler: 341,
            line: false,
        }
    }

    fn write_control(&mut self, val: u8) {
        self.enable_after_ack = val & 1 != 0;
        self.enabled = val & 2 != 0;
        self.cycle_mode = val & 4 != 0;
        self.line = false;
        if self.enabled {
            self.counter = self.latch;
            self.prescaler = 341;
        }
    }

    fn ack(&mut self) {
        self.line = false;
        self.enabled = self.enable_after_ack;
    }

    fn clock(&mut self) {
        if !self.enabled {
            return;
        }
        if !self.cycle_mode {
            // Scanline mode: one step per 113.667 CPU cycles.
            self.prescaler -= 3;
            if self.prescaler > 0 {
                return;
            }
            self.prescaler += 341;
        }
        if self.counter == 0xFF {
            self.counter = self.latch;
            self.line = true;
        } else {
            self.counter += 1;
        }
    }
}

struct Vrc6Pulse {
    // $x000: bits 0-3 volume, 4-6 duty, 7 constant-output mode.
    ctrl: u8,
    period: u16,
    enabled: bool,
    counter: u16,
    step: u8,
}

impl Vrc6Pulse {
    fn output(&self) -> u8 {
        if !self.enabled {
            0
        } else if self.ctrl & 0x80 != 0 || self.step <= (self.ctrl >> 4) & 7 {
            self.ctrl & 0x0F
        } else {
            0
        }
    }
}

struct Vrc6Audio {
    pulses: [Vrc6Pulse; 2],
    saw_rate: u8,
    saw_period: u16,
    saw_enabled: bool,
    saw_counter: u16,
    saw_step: u8,
    saw_acc: u8,
    // $9003: bit 0 halts all channels, bits 1-2 shift the periods.
    freq_ctrl: u8,
}

impl Vrc6Audio {
    fn new() -> Self {
        Vrc6Audio {
            pulses: [
                Vrc6Pulse { ctrl: 0, period: 0, enabled: false, counter: 0, step: 0 },
                Vrc6Pulse { ctrl: 0, period: 0, enabled: false, counter: 0, step: 0 },
            ],
            saw_rate: 0,
            saw_period: 0,
            saw_enabled: false,
            saw_counter: 0,
            saw_step: 0,
            saw_acc: 0,
            freq_ctrl: 0,
        }
    }

    fn pulse_write(&mut self, ch: usize, reg: u16, val: u8) {
        let p = &mut self.pulses[ch];
        match reg {
            0 => p.ctrl = val,
            1 => p.period = (p.period & 0xF00) | val as u16,
            _ => {
                p.period = (p.period & 0x0FF) | ((val as u16 & 0x0F) << 8);
                p.enabled = val & 0x80 != 0;
                if !p.enabled {
                    p.step = 0;
                }
            }
        }
    }

    fn saw_write(&mut self, reg: u16, val: u8) {
        match reg {
            0 => self.saw_rate = val & 0x3F,
            1 => self.saw_period = (self.saw_period & 0xF00) | val as u16,
            _ => {
                self.saw_period = (self.saw_period & 0x0FF) | ((val as u16 & 0x0F) << 8);
                self.saw_enabled = val & 0x80 != 0;
                if !self.saw_enabled {
                    self.saw_acc = 0;
                    self.saw_step = 0;
                }
            }
        }
    }

    fn effective_period(&self, period: u16) -> u16 {
        if self.freq_ctrl & 4 != 0 {
            period >> 8
        } else if self.freq_ctrl & 2 != 0 {
            period >> 4
        } else {
            period
        }
    }

    fn clock(&mut self) {
        if self.freq_ctrl & 1 != 0 {
            return; // halted
        }
        for ch in 0..2 {
            let eff = self.effective_period(self.pulses[ch].period);
            let p = &mut self.pulses[ch];
            if !p.enabled {
                continue;
            }
            if p.counter == 0 {
                p.counter = eff;
                p.step = (p.step + 1) & 15;
            } else {
                p.counter -= 1;
            }
        }
        if self.saw_enabled {
            if self.saw_counter == 0 {
                self.saw_counter = self.effective_period(self.saw_period);
                self.saw_step += 1;
                // The accumulator adds on every other step; 14 steps per cycle.
                if self.saw_step & 1 == 0 {
                    self.saw_acc = self.saw_acc.wrapping_add(self.saw_rate);
                }
                if self.saw_step == 14 {
                    self.saw_step = 0;
                    self.saw_acc = 0;
                }
            } else {
                self.saw_counter -= 1;
            }
        }
    }

    fn sample(&self) -> f32 {
        let p = self.pulses[0].output() + self.pulses[1].output();
        let saw = self.saw_acc >> 3; // top 5 bits
        (p + saw) as f32 * 0.00752
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vrc6() -> Vrc6 {
        // 8 x 16KB PRG, 16 x 1KB CHR; byte = bank index.
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Vrc6::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_banking() {
        let mut m = vrc6();
        m.cpu_write(0x8000, 2);
        m.cpu_write(0xC000, 9); // 8KB banks: 16KB bank 4 second half
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xC000), 4); // 8KB bank 9 lives in 16KB bank 4
        assert_eq!(m.cpu_read(0xE000), 7); // fixed last
    }

    #[test]
    fn chr_banking() {
        let mut m = vrc6();
        m.cpu_write(0xD002, 11);
        m.cpu_write(0xE001, 3);
        assert_eq!(m.ppu_read(0x0800), 11);
        assert_eq!(m.ppu_read(0x1400), 3);
    }

    #[test]
    fn mirroring_bits() {
        let mut m = vrc6();
        m.cpu_write(0xB003, 1 << 2);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0xB003, 2 << 2);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
    }

    #[test]
    fn irq_cycle_mode() {
        let mut m = vrc6();
        m.cpu_write(0xF000, 0xFD); // latch: 3 steps to $FF
        m.cpu_write(0xF001, 0x06); // enable, cycle mode
        for i in 0..2 {
            m.cpu_clock();
            assert!(!m.irq(), "IRQ too early at cycle {i}");
        }
        m.cpu_clock(); // $FF -> reload, IRQ
        assert!(m.irq());
        // $F002 acks; enable-after-ack clear stops the counter.
        m.cpu_write(0xF002, 0);
        assert!(!m.irq());
        for _ in 0..1000 {
            m.cpu_clock();
        }
        assert!(!m.irq());
    }

    #[test]
    fn irq_scanline_mode_cadence() {
        let mut m = vrc6();
        m.cpu_write(0xF000, 0xFF); // IRQ on first counter step
        m.cpu_write(0xF001, 0x02); // enable, scanline mode
        // First step lands after ceil(341/3) = 114 CPU cycles.
        for _ in 0..113 {
            m.cpu_clock();
        }
        assert!(!m.irq());
        m.cpu_clock();
        assert!(m.irq());
    }

    #[test]
    fn pulse_and_saw_output() {
        let mut m = vrc6();
        m.cpu_write(0x9000, 0x0F); // duty 0, volume 15
        m.cpu_write(0x9001, 0x04);
        m.cpu_write(0x9002, 0x80); // enable
        m.cpu_write(0xB000, 0x20); // saw rate
        m.cpu_write(0xB001, 0x04);
        m.cpu_write(0xB002, 0x80); // enable
        let mut peak = 0.0f32;
        for _ in 0..500 {
            m.cpu_clock();
            peak = peak.max(m.audio_sample());
        }
        assert!(peak > 0.05, "expected VRC6 audio output, got {peak}");
        // Halt bit freezes the channels.
        m.cpu_write(0x9003, 0x01);
        let s0 = m.audio_sample();
        for _ in 0..100 {
            m.cpu_clock();
        }
        assert_eq!(m.audio_sample(), s0);
    }
}
