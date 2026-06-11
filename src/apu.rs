//! NTSC APU, ticked once per CPU cycle (~1.789773 MHz).
//!
//! Implements the five channels (pulse 1/2, triangle, noise, DMC), the frame
//! counter with both sequencer modes and IRQ timing, the non-linear mixer,
//! and downsampling to the host sample rate (boxcar decimation followed by
//! the NES's analog filter chain: 90 Hz and 440 Hz high-pass, 14 kHz low-pass).

const CPU_HZ: f64 = 1_789_772.727;

const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, //
    12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
];

const DUTY: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];

const TRIANGLE_SEQ: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, //
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

// NTSC noise timer periods, in CPU cycles.
const NOISE_PERIOD: [u16; 16] =
    [4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068];

// NTSC DMC timer periods, in CPU cycles.
const DMC_RATE: [u16; 16] =
    [428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54];

#[derive(Default)]
struct Envelope {
    start: bool,
    loop_flag: bool, // doubles as the length counter halt flag
    constant: bool,
    period: u8,
    divider: u8,
    decay: u8,
}

impl Envelope {
    fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.decay = 15;
            self.divider = self.period;
        } else if self.divider == 0 {
            self.divider = self.period;
            if self.decay > 0 {
                self.decay -= 1;
            } else if self.loop_flag {
                self.decay = 15;
            }
        } else {
            self.divider -= 1;
        }
    }

    fn volume(&self) -> u8 {
        if self.constant { self.period } else { self.decay }
    }
}

struct Pulse {
    enabled: bool,
    duty: u8,
    seq_pos: u8,
    timer_period: u16,
    timer: u16,
    length: u8,
    env: Envelope,
    sweep_enabled: bool,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_divider: u8,
    sweep_reload: bool,
    ones_complement: bool, // pulse 1 negates with ones' complement
}

impl Pulse {
    fn new(ones_complement: bool) -> Self {
        Pulse {
            enabled: false,
            duty: 0,
            seq_pos: 0,
            timer_period: 0,
            timer: 0,
            length: 0,
            env: Envelope::default(),
            sweep_enabled: false,
            sweep_period: 0,
            sweep_negate: false,
            sweep_shift: 0,
            sweep_divider: 0,
            sweep_reload: false,
            ones_complement,
        }
    }

    fn sweep_target(&self) -> i32 {
        let change = (self.timer_period >> self.sweep_shift) as i32;
        if self.sweep_negate {
            self.timer_period as i32 - change - self.ones_complement as i32
        } else {
            self.timer_period as i32 + change
        }
    }

    /// Sweep muting applies even when the sweep unit is disabled.
    fn muted(&self) -> bool {
        self.timer_period < 8 || self.sweep_target() > 0x7FF
    }

    fn clock_sweep(&mut self) {
        if self.sweep_divider == 0 && self.sweep_enabled && self.sweep_shift != 0 && !self.muted()
        {
            self.timer_period = self.sweep_target().max(0) as u16;
        }
        if self.sweep_divider == 0 || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
    }

    fn clock_length(&mut self) {
        if self.length > 0 && !self.env.loop_flag {
            self.length -= 1;
        }
    }

    /// Clocked every other CPU cycle (APU cycle).
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            self.seq_pos = (self.seq_pos + 1) & 7;
        } else {
            self.timer -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.length == 0 || self.muted() || DUTY[self.duty as usize][self.seq_pos as usize] == 0
        {
            0
        } else {
            self.env.volume()
        }
    }

    fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.length = 0;
        }
    }
}

#[derive(Default)]
struct Triangle {
    enabled: bool,
    control: bool, // length counter halt / linear counter control
    linear_reload_value: u8,
    linear: u8,
    linear_reload: bool,
    timer_period: u16,
    timer: u16,
    length: u8,
    seq_pos: u8,
}

impl Triangle {
    /// Clocked every CPU cycle.
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            if self.length > 0 && self.linear > 0 {
                self.seq_pos = (self.seq_pos + 1) & 31;
            }
        } else {
            self.timer -= 1;
        }
    }

    fn clock_linear(&mut self) {
        if self.linear_reload {
            self.linear = self.linear_reload_value;
        } else if self.linear > 0 {
            self.linear -= 1;
        }
        if !self.control {
            self.linear_reload = false;
        }
    }

    fn clock_length(&mut self) {
        if self.length > 0 && !self.control {
            self.length -= 1;
        }
    }

    /// The triangle keeps outputting its current sequencer value when halted.
    fn output(&self) -> u8 {
        TRIANGLE_SEQ[self.seq_pos as usize]
    }

    fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.length = 0;
        }
    }
}

struct Noise {
    enabled: bool,
    mode: bool, // bit 6 tap when set (short loop), bit 1 otherwise
    timer_period: u16,
    timer: u16,
    shift: u16,
    length: u8,
    env: Envelope,
}

impl Noise {
    fn new() -> Self {
        Noise {
            enabled: false,
            mode: false,
            timer_period: NOISE_PERIOD[0],
            timer: 0,
            shift: 1,
            length: 0,
            env: Envelope::default(),
        }
    }

    /// Clocked every CPU cycle (period table is in CPU cycles).
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period - 1;
            let tap = if self.mode { 6 } else { 1 };
            let feedback = (self.shift ^ (self.shift >> tap)) & 1;
            self.shift = (self.shift >> 1) | (feedback << 14);
        } else {
            self.timer -= 1;
        }
    }

    fn clock_length(&mut self) {
        if self.length > 0 && !self.env.loop_flag {
            self.length -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.length == 0 || self.shift & 1 == 1 { 0 } else { self.env.volume() }
    }

    fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.length = 0;
        }
    }
}

struct Dmc {
    irq_enabled: bool,
    loop_flag: bool,
    period: u16,
    timer: u16,
    level: u8,
    sample_addr: u16,
    sample_len: u16,
    current_addr: u16,
    bytes_remaining: u16,
    shift: u8,
    bits_remaining: u8,
    silence: bool,
    buffer: Option<u8>,
    fetch_pending: bool,
    irq: bool,
}

impl Dmc {
    fn new() -> Self {
        Dmc {
            irq_enabled: false,
            loop_flag: false,
            period: DMC_RATE[0],
            timer: 0,
            level: 0,
            sample_addr: 0xC000,
            sample_len: 1,
            current_addr: 0xC000,
            bytes_remaining: 0,
            shift: 0,
            bits_remaining: 8,
            silence: true,
            buffer: None,
            fetch_pending: false,
            irq: false,
        }
    }

    fn restart(&mut self) {
        self.current_addr = self.sample_addr;
        self.bytes_remaining = self.sample_len;
    }

    /// Clocked every CPU cycle.
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.period - 1;
            self.clock_output();
        } else {
            self.timer -= 1;
        }
    }

    fn clock_output(&mut self) {
        if !self.silence {
            if self.shift & 1 == 1 {
                if self.level <= 125 {
                    self.level += 2;
                }
            } else if self.level >= 2 {
                self.level -= 2;
            }
        }
        self.shift >>= 1;
        self.bits_remaining -= 1;
        if self.bits_remaining == 0 {
            self.bits_remaining = 8;
            match self.buffer.take() {
                Some(b) => {
                    self.silence = false;
                    self.shift = b;
                }
                None => self.silence = true,
            }
        }
    }

    /// The DMA unit delivers the fetched sample byte.
    fn supply(&mut self, v: u8) {
        self.buffer = Some(v);
        self.fetch_pending = false;
        self.current_addr =
            if self.current_addr == 0xFFFF { 0x8000 } else { self.current_addr + 1 };
        self.bytes_remaining -= 1;
        if self.bytes_remaining == 0 {
            if self.loop_flag {
                self.restart();
            } else if self.irq_enabled {
                self.irq = true;
            }
        }
    }
}

struct HighPass {
    a: f32,
    prev_in: f32,
    prev_out: f32,
}

impl HighPass {
    fn new(fc: f64, rate: f64) -> Self {
        let rc = 1.0 / (2.0 * std::f64::consts::PI * fc);
        HighPass { a: (rc / (rc + 1.0 / rate)) as f32, prev_in: 0.0, prev_out: 0.0 }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.a * (self.prev_out + x - self.prev_in);
        self.prev_in = x;
        self.prev_out = y;
        y
    }
}

struct LowPass {
    b: f32,
    prev: f32,
}

impl LowPass {
    fn new(fc: f64, rate: f64) -> Self {
        let rc = 1.0 / (2.0 * std::f64::consts::PI * fc);
        let dt = 1.0 / rate;
        LowPass { b: (dt / (rc + dt)) as f32, prev: 0.0 }
    }

    fn process(&mut self, x: f32) -> f32 {
        self.prev += self.b * (x - self.prev);
        self.prev
    }
}

pub struct Apu {
    pulse1: Pulse,
    pulse2: Pulse,
    triangle: Triangle,
    noise: Noise,
    dmc: Dmc,

    cycle: u64, // total CPU cycles, used for $4017 write-delay parity
    frame_cycle: u32,
    frame_mode5: bool,
    pending_mode5: bool,
    irq_inhibit: bool,
    frame_irq: bool,
    frame_reset_delay: u8,

    pulse_table: [f32; 31],
    tnd_table: [f32; 203],

    cycles_per_sample: f64,
    sample_frac: f64,
    acc: f32,
    acc_n: u32,
    samples: Vec<f32>,
    hp1: HighPass,
    hp2: HighPass,
    lp: LowPass,
}

impl Apu {
    pub fn new() -> Self {
        let mut pulse_table = [0f32; 31];
        for (n, e) in pulse_table.iter_mut().enumerate().skip(1) {
            *e = 95.52 / (8128.0 / n as f32 + 100.0);
        }
        let mut tnd_table = [0f32; 203];
        for (n, e) in tnd_table.iter_mut().enumerate().skip(1) {
            *e = 163.67 / (24329.0 / n as f32 + 100.0);
        }
        let rate = 48_000.0;
        Apu {
            pulse1: Pulse::new(true),
            pulse2: Pulse::new(false),
            triangle: Triangle::default(),
            noise: Noise::new(),
            dmc: Dmc::new(),
            cycle: 0,
            frame_cycle: 0,
            frame_mode5: false,
            pending_mode5: false,
            irq_inhibit: false,
            frame_irq: false,
            frame_reset_delay: 0,
            pulse_table,
            tnd_table,
            cycles_per_sample: CPU_HZ / rate,
            sample_frac: 0.0,
            acc: 0.0,
            acc_n: 0,
            samples: Vec::new(),
            hp1: HighPass::new(90.0, rate),
            hp2: HighPass::new(440.0, rate),
            lp: LowPass::new(14_000.0, rate),
        }
    }

    /// Set the host output rate; resets the filter chain.
    pub fn set_sample_rate(&mut self, hz: f64) {
        self.cycles_per_sample = CPU_HZ / hz;
        self.hp1 = HighPass::new(90.0, hz);
        self.hp2 = HighPass::new(440.0, hz);
        self.lp = LowPass::new(14_000.0, hz);
    }

    /// Nudge the resampling ratio for dynamic rate control (keeps filters).
    pub fn tune(&mut self, hz: f64) {
        self.cycles_per_sample = CPU_HZ / hz;
    }

    pub fn take_samples(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.samples)
    }

    /// Frame IRQ and DMC IRQ are level-triggered.
    pub fn irq(&self) -> bool {
        self.frame_irq || self.dmc.irq
    }

    pub fn read_status(&mut self) -> u8 {
        let mut v = 0;
        if self.pulse1.length > 0 {
            v |= 0x01;
        }
        if self.pulse2.length > 0 {
            v |= 0x02;
        }
        if self.triangle.length > 0 {
            v |= 0x04;
        }
        if self.noise.length > 0 {
            v |= 0x08;
        }
        if self.dmc.bytes_remaining > 0 {
            v |= 0x10;
        }
        if self.frame_irq {
            v |= 0x40;
        }
        if self.dmc.irq {
            v |= 0x80;
        }
        // reading $4015 clears the frame IRQ flag (but not the DMC IRQ)
        self.frame_irq = false;
        v
    }

    pub fn write(&mut self, addr: u16, v: u8) {
        match addr {
            0x4000 | 0x4004 => {
                let p = if addr == 0x4000 { &mut self.pulse1 } else { &mut self.pulse2 };
                p.duty = v >> 6;
                p.env.loop_flag = v & 0x20 != 0;
                p.env.constant = v & 0x10 != 0;
                p.env.period = v & 0x0F;
            }
            0x4001 | 0x4005 => {
                let p = if addr == 0x4001 { &mut self.pulse1 } else { &mut self.pulse2 };
                p.sweep_enabled = v & 0x80 != 0;
                p.sweep_period = (v >> 4) & 7;
                p.sweep_negate = v & 0x08 != 0;
                p.sweep_shift = v & 7;
                p.sweep_reload = true;
            }
            0x4002 | 0x4006 => {
                let p = if addr == 0x4002 { &mut self.pulse1 } else { &mut self.pulse2 };
                p.timer_period = (p.timer_period & 0x0700) | v as u16;
            }
            0x4003 | 0x4007 => {
                let p = if addr == 0x4003 { &mut self.pulse1 } else { &mut self.pulse2 };
                p.timer_period = (p.timer_period & 0x00FF) | (((v & 7) as u16) << 8);
                if p.enabled {
                    p.length = LENGTH_TABLE[(v >> 3) as usize];
                }
                p.seq_pos = 0; // phase reset
                p.env.start = true;
            }
            0x4008 => {
                self.triangle.control = v & 0x80 != 0;
                self.triangle.linear_reload_value = v & 0x7F;
            }
            0x400A => {
                self.triangle.timer_period = (self.triangle.timer_period & 0x0700) | v as u16;
            }
            0x400B => {
                self.triangle.timer_period =
                    (self.triangle.timer_period & 0x00FF) | (((v & 7) as u16) << 8);
                if self.triangle.enabled {
                    self.triangle.length = LENGTH_TABLE[(v >> 3) as usize];
                }
                self.triangle.linear_reload = true;
            }
            0x400C => {
                self.noise.env.loop_flag = v & 0x20 != 0;
                self.noise.env.constant = v & 0x10 != 0;
                self.noise.env.period = v & 0x0F;
            }
            0x400E => {
                self.noise.mode = v & 0x80 != 0;
                self.noise.timer_period = NOISE_PERIOD[(v & 0x0F) as usize];
            }
            0x400F => {
                if self.noise.enabled {
                    self.noise.length = LENGTH_TABLE[(v >> 3) as usize];
                }
                self.noise.env.start = true;
            }
            0x4010 => {
                self.dmc.irq_enabled = v & 0x80 != 0;
                if !self.dmc.irq_enabled {
                    self.dmc.irq = false;
                }
                self.dmc.loop_flag = v & 0x40 != 0;
                self.dmc.period = DMC_RATE[(v & 0x0F) as usize];
            }
            0x4011 => self.dmc.level = v & 0x7F,
            0x4012 => self.dmc.sample_addr = 0xC000 | ((v as u16) << 6),
            0x4013 => self.dmc.sample_len = ((v as u16) << 4) | 1,
            0x4015 => {
                self.pulse1.set_enabled(v & 0x01 != 0);
                self.pulse2.set_enabled(v & 0x02 != 0);
                self.triangle.set_enabled(v & 0x04 != 0);
                self.noise.set_enabled(v & 0x08 != 0);
                self.dmc.irq = false;
                if v & 0x10 != 0 {
                    if self.dmc.bytes_remaining == 0 {
                        self.dmc.restart();
                    }
                } else {
                    self.dmc.bytes_remaining = 0;
                }
            }
            0x4017 => {
                self.pending_mode5 = v & 0x80 != 0;
                self.irq_inhibit = v & 0x40 != 0;
                if self.irq_inhibit {
                    self.frame_irq = false;
                }
                // sequencer reset lands 3 or 4 cycles later depending on parity
                self.frame_reset_delay = if self.cycle & 1 == 0 { 3 } else { 4 };
            }
            _ => {}
        }
    }

    /// Advance one CPU cycle. Returns `Some(addr)` when the DMC DMA unit
    /// needs a sample byte fetched from memory (deliver it via `dmc_supply`).
    pub fn tick(&mut self) -> Option<u16> {
        self.cycle += 1;

        if self.frame_reset_delay > 0 {
            self.frame_reset_delay -= 1;
            if self.frame_reset_delay == 0 {
                self.frame_cycle = 0;
                self.frame_mode5 = self.pending_mode5;
                if self.frame_mode5 {
                    self.clock_quarter();
                    self.clock_half();
                }
            }
        } else {
            self.frame_cycle += 1;
            self.clock_frame_events();
        }

        if self.cycle & 1 == 0 {
            self.pulse1.clock_timer();
            self.pulse2.clock_timer();
        }
        self.triangle.clock_timer();
        self.noise.clock_timer();
        self.dmc.clock_timer();

        let mut fetch = None;
        if self.dmc.buffer.is_none() && self.dmc.bytes_remaining > 0 && !self.dmc.fetch_pending {
            self.dmc.fetch_pending = true;
            fetch = Some(self.dmc.current_addr);
        }

        // boxcar decimation to the output rate, then the analog filter chain
        self.acc += self.mix();
        self.acc_n += 1;
        self.sample_frac += 1.0;
        if self.sample_frac >= self.cycles_per_sample {
            self.sample_frac -= self.cycles_per_sample;
            let s = self.acc / self.acc_n as f32;
            self.acc = 0.0;
            self.acc_n = 0;
            let f = self.lp.process(self.hp2.process(self.hp1.process(s)));
            self.samples.push(f);
        }
        fetch
    }

    pub fn dmc_supply(&mut self, v: u8) {
        self.dmc.supply(v);
    }

    fn mix(&self) -> f32 {
        let p = (self.pulse1.output() + self.pulse2.output()) as usize;
        let tnd = 3 * self.triangle.output() as usize
            + 2 * self.noise.output() as usize
            + self.dmc.level as usize;
        self.pulse_table[p] + self.tnd_table[tnd]
    }

    fn set_frame_irq(&mut self) {
        if !self.irq_inhibit {
            self.frame_irq = true;
        }
    }

    // NTSC frame counter, counted in CPU cycles (events at APU half-cycles).
    fn clock_frame_events(&mut self) {
        if !self.frame_mode5 {
            match self.frame_cycle {
                7457 => self.clock_quarter(),
                14913 => {
                    self.clock_quarter();
                    self.clock_half();
                }
                22371 => self.clock_quarter(),
                29828 => self.set_frame_irq(),
                29829 => {
                    self.clock_quarter();
                    self.clock_half();
                    self.set_frame_irq();
                }
                29830 => {
                    self.set_frame_irq();
                    self.frame_cycle = 0;
                }
                _ => {}
            }
        } else {
            match self.frame_cycle {
                7457 => self.clock_quarter(),
                14913 => {
                    self.clock_quarter();
                    self.clock_half();
                }
                22371 => self.clock_quarter(),
                37281 => {
                    self.clock_quarter();
                    self.clock_half();
                }
                37282 => self.frame_cycle = 0,
                _ => {}
            }
        }
    }

    fn clock_quarter(&mut self) {
        self.pulse1.env.clock();
        self.pulse2.env.clock();
        self.noise.env.clock();
        self.triangle.clock_linear();
    }

    fn clock_half(&mut self) {
        self.pulse1.clock_length();
        self.pulse1.clock_sweep();
        self.pulse2.clock_length();
        self.pulse2.clock_sweep();
        self.triangle.clock_length();
        self.noise.clock_length();
    }
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(apu: &mut Apu, cycles: u32) {
        for _ in 0..cycles {
            apu.tick();
        }
    }

    #[test]
    fn frame_irq_fires_in_mode_0() {
        let mut apu = Apu::new();
        run(&mut apu, 29827);
        assert!(!apu.irq(), "IRQ before sequence end");
        run(&mut apu, 1); // flag window starts at CPU cycle 29828
        assert!(apu.irq(), "IRQ at sequence end");
        run(&mut apu, 3); // get past the 29828-29830 window where the flag re-sets
        // reading $4015 reports and clears the flag
        assert_eq!(apu.read_status() & 0x40, 0x40);
        assert!(!apu.irq());
    }

    #[test]
    fn mode_5_has_no_irq() {
        let mut apu = Apu::new();
        apu.write(0x4017, 0x80);
        run(&mut apu, 40_000);
        assert!(!apu.irq());
    }

    #[test]
    fn irq_inhibit_clears_flag() {
        let mut apu = Apu::new();
        run(&mut apu, 30_000);
        assert!(apu.irq());
        apu.write(0x4017, 0x40);
        assert!(!apu.irq());
        run(&mut apu, 40_000);
        assert!(!apu.irq());
    }

    #[test]
    fn length_counter_loads_and_counts_down() {
        let mut apu = Apu::new();
        apu.write(0x4015, 0x01);
        apu.write(0x4017, 0x40); // inhibit IRQ so the test only sees lengths
        apu.write(0x4000, 0x00); // halt clear
        apu.write(0x4003, 0x18); // length index 3 -> 2
        assert_eq!(apu.read_status() & 1, 1);
        // two half-frame clocks (at 14913 and 29829, plus the 3-4 cycle
        // $4017 reset delay) expire a length of 2
        run(&mut apu, 29_840);
        assert_eq!(apu.read_status() & 1, 0);
    }

    #[test]
    fn disabled_channel_does_not_load_length() {
        let mut apu = Apu::new();
        apu.write(0x4003, 0x18);
        assert_eq!(apu.read_status() & 1, 0);
    }

    #[test]
    fn pulse_produces_audio() {
        let mut apu = Apu::new();
        apu.write(0x4015, 0x01);
        apu.write(0x4000, 0xBF); // duty 2, halt, constant volume 15
        apu.write(0x4002, 0xFD); // ~440 Hz
        apu.write(0x4003, 0x08);
        run(&mut apu, 100_000);
        let s = apu.take_samples();
        assert!(!s.is_empty());
        assert!(s.iter().any(|v| v.abs() > 0.01), "expected audible output");
    }

    #[test]
    fn dmc_requests_fetches_and_raises_irq() {
        let mut apu = Apu::new();
        apu.write(0x4010, 0x8F); // IRQ on, slowest rate
        apu.write(0x4012, 0x00); // addr $C000
        apu.write(0x4013, 0x00); // length 1 byte
        apu.write(0x4015, 0x10);
        let mut fetched = 0;
        for _ in 0..10_000 {
            if let Some(addr) = apu.tick() {
                assert_eq!(addr, 0xC000);
                apu.dmc_supply(0xFF);
                fetched += 1;
            }
        }
        assert_eq!(fetched, 1);
        assert!(apu.irq(), "DMC IRQ after last byte");
        apu.write(0x4015, 0x00);
        assert!(!apu.irq(), "$4015 write clears DMC IRQ");
    }

    #[test]
    fn sweep_mutes_low_periods() {
        let mut apu = Apu::new();
        apu.write(0x4015, 0x01);
        apu.write(0x4000, 0x3F);
        apu.write(0x4002, 0x04); // period 4 < 8 -> muted
        apu.write(0x4003, 0x08);
        run(&mut apu, 50_000);
        let s = apu.take_samples();
        // skip the initial high-pass transient from the idle triangle DC level
        assert!(s[500..].iter().all(|v| v.abs() < 0.005), "muted pulse leaked audio");
    }
}
