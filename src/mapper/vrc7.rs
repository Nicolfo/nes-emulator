use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// VRC7 (mapper 85, Lagrange Point): three switchable 8KB PRG banks plus a
/// fixed last bank, eight 1KB CHR banks, 8KB battery PRG RAM, the shared VRC
/// scanline/cycle IRQ, and the Yamaha-OPLL-derived FM expansion audio (six
/// two-operator channels, 15 built-in instruments plus one programmable one).
///
/// Two board pinouts decode the register-select line differently: VRC7a
/// (submapper 2, Lagrange Point) uses A4 ($x010), VRC7b (submapper 1, Tiny
/// Toon Adventures 2) uses A3 ($x008). The sound ports add A5.
#[derive(Clone, Serialize, Deserialize)]
pub struct Vrc7 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    #[serde(with = "crate::savestate::byte_array")]
    prg_ram: [u8; 0x2000],
    mirroring: Mirroring,
    prg_banks: [u8; 3], // 8KB banks at $8000/$A000/$C000 ($E000 is fixed)
    chr_banks: [u8; 8], // 1KB banks across $0000-$1FFF
    irq: VrcIrq,
    audio: Opll,
    /// Register-select line: 0x10 for VRC7a (A4), 0x08 for VRC7b (A3).
    sel: u16,
}

impl Vrc7 {
    pub fn new(submapper: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        // Submapper 1 = VRC7b (A3 select); everything else = VRC7a (A4 select).
        let sel = if submapper == 1 { 0x08 } else { 0x10 };
        // Lagrange Point ships 8KB CHR RAM (no CHR ROM in the header).
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Vrc7 {
            prg,
            chr,
            chr_is_ram,
            prg_ram: [0; 0x2000],
            mirroring,
            prg_banks: [0; 3],
            chr_banks: [0; 8],
            irq: VrcIrq::new(),
            audio: Opll::new(),
            sel,
        }
    }

    fn prg_offset(&self, bank: usize, addr: u16) -> usize {
        let banks = (self.prg.len() / 0x2000).max(1);
        (bank % banks) * 0x2000 + (addr as usize & 0x1FFF)
    }
}

impl Mapper for Vrc7 {
    crate::impl_mapper_savestate!(chr_is_ram = chr_is_ram);

    fn cpu_read(&mut self, addr: u16) -> u8 {
        match addr {
            0x8000..=0x9FFF => self.prg[self.prg_offset(self.prg_banks[0] as usize, addr)],
            0xA000..=0xBFFF => self.prg[self.prg_offset(self.prg_banks[1] as usize, addr)],
            0xC000..=0xDFFF => self.prg[self.prg_offset(self.prg_banks[2] as usize, addr)],
            0xE000..=0xFFFF => {
                // Fixed last 8KB bank.
                let last = self.prg.len().saturating_sub(0x2000);
                self.prg[last + (addr as usize & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if (0x6000..0x8000).contains(&addr) {
            self.prg_ram[(addr & 0x1FFF) as usize] = val;
            return;
        }
        let sel = self.sel;
        match addr & 0xF000 {
            0x8000 => {
                if addr & sel == 0 {
                    self.prg_banks[0] = val & 0x3F;
                } else {
                    self.prg_banks[1] = val & 0x3F;
                }
            }
            0x9000 => {
                // $9000 PRG2, $x010/$x008 sound-address latch, $x030/$x028 data.
                if addr & 0x20 != 0 {
                    self.audio.write_data(val);
                } else if addr & sel != 0 {
                    self.audio.write_addr(val);
                } else {
                    self.prg_banks[2] = val & 0x3F;
                }
            }
            0xA000 => self.chr_banks[if addr & sel == 0 { 0 } else { 1 }] = val,
            0xB000 => self.chr_banks[if addr & sel == 0 { 2 } else { 3 }] = val,
            0xC000 => self.chr_banks[if addr & sel == 0 { 4 } else { 5 }] = val,
            0xD000 => self.chr_banks[if addr & sel == 0 { 6 } else { 7 }] = val,
            0xE000 => {
                if addr & sel == 0 {
                    if self.mirroring != Mirroring::FourScreen {
                        self.mirroring = match val & 3 {
                            0 => Mirroring::Vertical,
                            1 => Mirroring::Horizontal,
                            2 => Mirroring::SingleScreenLo,
                            _ => Mirroring::SingleScreenHi,
                        };
                    }
                    // Bit 7 silences the expansion sound; bit 6 is WRAM enable
                    // (PRG RAM writes are accepted unconditionally here).
                    self.audio.silence = val & 0x80 != 0;
                } else {
                    self.irq.latch = val;
                }
            }
            0xF000 => {
                if addr & sel == 0 {
                    self.irq.write_control(val);
                } else {
                    self.irq.ack();
                }
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let banks = (self.chr.len() / 0x400).max(1);
        let bank = self.chr_banks[(addr >> 10) as usize & 7] as usize % banks;
        self.chr[bank * 0x400 + (addr as usize & 0x3FF)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        // CHR ROM is read-only; CHR RAM boards (Lagrange Point) accept writes.
        if self.chr_is_ram {
            let banks = (self.chr.len() / 0x400).max(1);
            let bank = self.chr_banks[(addr >> 10) as usize & 7] as usize % banks;
            self.chr[bank * 0x400 + (addr as usize & 0x3FF)] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        Some(self.prg_ram[(addr & 0x1FFF) as usize])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.prg_ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.prg_ram)
    }

    fn irq(&self) -> bool {
        self.irq.line
    }

    fn cpu_clock(&mut self) {
        self.irq.clock();
        self.audio.clock();
    }

    fn audio_sample(&self) -> f32 {
        self.audio.output()
    }
}

/// The shared Konami VRC IRQ: an up-counter from a reloadable latch, in
/// CPU-cycle mode or scanline mode (a 341/3-dot prescaler). Identical to the
/// unit in VRC4/VRC6.
#[derive(Clone, Serialize, Deserialize)]
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

// ---------------------------------------------------------------------------
// OPLL (Yamaha YM2413 derivative) FM sound core.
//
// Six channels, two operators each (a modulator feeding a carrier). The core
// runs in the log/dB domain for level control - total level, channel volume,
// sustain level, key-scale level are all attenuations in dB - while phase and
// FM are computed with f64 sines. The instrument ROM, multiplier table, dB
// step sizes and the per-channel update rate (49716 Hz) follow the nesdev
// VRC7 audio reference. The ADSR *shape* is an exponential-segment
// approximation rather than a transcription of the chip's envelope tables, so
// timbre/pitch are faithful but envelope timing is approximate (the kind of
// thing the project verifies by ear against NSF recordings).
// ---------------------------------------------------------------------------

/// Per-channel update rate: the chip refreshes all channels every 72 cycles of
/// its 3.58 MHz clock (49716 Hz). The NES CPU runs at ~1.789773 MHz, so one
/// OPLL sample is generated every ~36 CPU cycles.
const CYCLES_PER_SAMPLE: u16 = 36;

/// Attenuation (dB) at or above which an operator is effectively silent. Also
/// the envelope's resting/maximum value.
const MAX_ATT: f64 = 48.0;

/// MULT register -> frequency multiplier (index 0 means x0.5).
const MULT: [f64; 16] = [
    0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 10.0, 12.0, 12.0, 15.0, 15.0,
];

/// Feedback level (FFF) -> self-modulation depth, in cycles per unit of the
/// (averaged) modulator output. The reference lists these in radians
/// (0, pi/16 .. 4pi); divided by 2pi they become these cycle fractions.
const FEEDBACK: [f64; 8] = [
    0.0,
    1.0 / 32.0,
    1.0 / 16.0,
    1.0 / 8.0,
    1.0 / 4.0,
    0.5,
    1.0,
    2.0,
];

/// Modulator-output -> carrier phase-modulation depth, in cycles (≈ pi index).
const FM_DEPTH: f64 = 0.5;

/// Overall output gain, chosen so a single full-volume channel sits near the
/// VRC6 expansion-audio level.
const GAIN: f32 = 0.06;

/// The 15 built-in instrument patches (8 bytes each), from the nesdev VRC7
/// audio reference. Instrument 0 is the programmable patch in `custom`.
const INSTRUMENTS: [[u8; 8]; 15] = [
    [0x03, 0x21, 0x05, 0x06, 0xE8, 0x81, 0x42, 0x27], // 1 Buzzy Bell
    [0x13, 0x41, 0x14, 0x0D, 0xD8, 0xF6, 0x23, 0x12], // 2 Guitar
    [0x11, 0x11, 0x08, 0x08, 0xFA, 0xB2, 0x20, 0x12], // 3 Wurly
    [0x31, 0x61, 0x0C, 0x07, 0xA8, 0x64, 0x61, 0x27], // 4 Flute
    [0x32, 0x21, 0x1E, 0x06, 0xE1, 0x76, 0x01, 0x28], // 5 Clarinet
    [0x02, 0x01, 0x06, 0x00, 0xA3, 0xE2, 0xF4, 0xF4], // 6 Synth
    [0x21, 0x61, 0x1D, 0x07, 0x82, 0x81, 0x11, 0x07], // 7 Trumpet
    [0x23, 0x21, 0x22, 0x17, 0xA2, 0x72, 0x01, 0x17], // 8 Organ
    [0x35, 0x11, 0x25, 0x00, 0x40, 0x73, 0x72, 0x01], // 9 Bells
    [0xB5, 0x01, 0x0F, 0x0F, 0xA8, 0xA5, 0x51, 0x02], // A Vibes
    [0x17, 0xC1, 0x24, 0x07, 0xF8, 0xF8, 0x22, 0x12], // B Vibraphone
    [0x71, 0x23, 0x11, 0x06, 0x65, 0x74, 0x18, 0x16], // C Tutti
    [0x01, 0x02, 0xD3, 0x05, 0xC9, 0x95, 0x03, 0x02], // D Fretless
    [0x61, 0x63, 0x0C, 0x00, 0x94, 0xC0, 0x33, 0xF6], // E Synth Bass
    [0x21, 0x72, 0x0D, 0x00, 0xC1, 0xD5, 0x56, 0x06], // F Sweep
];

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)]
enum Eg {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// One FM operator's live state.
#[derive(Serialize, Deserialize, Clone, Copy)]
struct Op {
    phase: f64, // 0..1 cycles
    env: f64,   // current envelope attenuation in dB (0 = loudest)
    state: Eg,
    out1: f64, // last two outputs, averaged for feedback
    out2: f64,
}

impl Op {
    fn new() -> Self {
        Op {
            phase: 0.0,
            env: MAX_ATT,
            state: Eg::Idle,
            out1: 0.0,
            out2: 0.0,
        }
    }

    fn key_on(&mut self) {
        self.state = Eg::Attack;
        self.phase = 0.0;
        self.out1 = 0.0;
        self.out2 = 0.0;
    }

    fn key_off(&mut self) {
        if self.state != Eg::Idle {
            self.state = Eg::Release;
        }
    }

    /// Advance the envelope one sample. `attack`/`decay`/`release` are dB steps
    /// already scaled for the effective rate; `sl_db` is the sustain level.
    fn eg_advance(&mut self, attack: f64, decay: f64, release: f64, sl_db: f64, sustaining: bool) {
        match self.state {
            Eg::Idle | Eg::Sustain => {}
            Eg::Attack => {
                self.env -= attack;
                if self.env <= 0.0 {
                    self.env = 0.0;
                    self.state = Eg::Decay;
                }
            }
            Eg::Decay => {
                self.env += decay;
                if self.env >= sl_db {
                    self.env = sl_db;
                    // Sustained tones hold at the sustain level; percussive
                    // ones keep decaying toward silence even with the key held.
                    self.state = if sustaining { Eg::Sustain } else { Eg::Release };
                }
            }
            Eg::Release => {
                self.env += release;
                if self.env >= MAX_ATT {
                    self.env = MAX_ATT;
                    self.state = Eg::Idle;
                }
            }
        }
    }
}

/// One channel's register-controlled configuration.
#[derive(Serialize, Deserialize, Clone, Copy)]
struct Channel {
    fnum: u16, // 9-bit
    block: u8, // 0-7
    key: bool,
    sustain: bool, // $2X bit 5: slow key-off release
    inst: u8,      // 0-15
    vol: u8,       // 0-15 (attenuation, 0 = loudest)
}

impl Channel {
    fn new() -> Self {
        Channel {
            fnum: 0,
            block: 0,
            key: false,
            sustain: false,
            inst: 0,
            vol: 0,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Opll {
    custom: [u8; 8], // programmable instrument (patch 0)
    ch: [Channel; 6],
    modu: [Op; 6], // modulators
    car: [Op; 6],  // carriers
    addr: u8,      // latched sound-register address
    lfo_am: f64,   // 0..1 tremolo phase
    lfo_pm: f64,   // 0..1 vibrato phase
    timer: u16,    // CPU cycles since the last generated sample
    out: f32,      // last generated sample
    pub silence: bool,
}

impl Opll {
    fn new() -> Self {
        Opll {
            custom: [0; 8],
            ch: [Channel::new(); 6],
            modu: [Op::new(); 6],
            car: [Op::new(); 6],
            addr: 0,
            lfo_am: 0.0,
            lfo_pm: 0.0,
            timer: 0,
            out: 0.0,
            silence: false,
        }
    }

    fn write_addr(&mut self, val: u8) {
        self.addr = val;
    }

    fn write_data(&mut self, val: u8) {
        match self.addr {
            0x00..=0x07 => self.custom[self.addr as usize] = val,
            0x10..=0x15 => {
                let c = (self.addr - 0x10) as usize;
                self.ch[c].fnum = (self.ch[c].fnum & 0x100) | val as u16;
            }
            0x20..=0x25 => {
                let c = (self.addr - 0x20) as usize;
                self.ch[c].fnum = (self.ch[c].fnum & 0xFF) | ((val as u16 & 1) << 8);
                self.ch[c].block = (val >> 1) & 7;
                self.ch[c].sustain = val & 0x20 != 0;
                let key = val & 0x10 != 0;
                if key && !self.ch[c].key {
                    self.modu[c].key_on();
                    self.car[c].key_on();
                } else if !key && self.ch[c].key {
                    self.modu[c].key_off();
                    self.car[c].key_off();
                }
                self.ch[c].key = key;
            }
            0x30..=0x35 => {
                let c = (self.addr - 0x30) as usize;
                self.ch[c].inst = val >> 4;
                self.ch[c].vol = val & 0x0F;
            }
            _ => {} // $0E rhythm / $0F test: unused by VRC7 melodic playback
        }
    }

    fn clock(&mut self) {
        self.timer += 1;
        if self.timer >= CYCLES_PER_SAMPLE {
            self.timer -= CYCLES_PER_SAMPLE;
            self.run_sample();
        }
    }

    fn output(&self) -> f32 {
        self.out
    }

    fn patch(&self, c: usize) -> [u8; 8] {
        let inst = self.ch[c].inst;
        if inst == 0 {
            self.custom
        } else {
            INSTRUMENTS[(inst - 1) as usize]
        }
    }

    fn run_sample(&mut self) {
        // Global low-frequency oscillators (shared by all operators).
        self.lfo_am = (self.lfo_am + 3.7 / 49716.0).fract();
        self.lfo_pm = (self.lfo_pm + 6.4 / 49716.0).fract();
        let am_sin = (self.lfo_am * std::f64::consts::TAU).sin();
        let pm_sin = (self.lfo_pm * std::f64::consts::TAU).sin();

        let mut mix = 0.0;
        for c in 0..6 {
            let p = self.patch(c);
            let ch = self.ch[c];
            let ksn = ((ch.block as i32) << 1) | ((ch.fnum >> 8) & 1) as i32;

            // --- modulator operator (patch bytes 0,2,4,6) ---
            let m = OpParams::decode(&p, false);
            let m_rks = if m.ksr { ksn } else { ksn >> 2 };
            let m_inc = phase_inc(ch.fnum, ch.block, m.mult, m.vib, pm_sin);
            let m_att =
                m.tl_db + ksl_db(m.ksl, ch.block, ch.fnum) + self.modu[c].env + m.am_db(am_sin);
            let fb = FEEDBACK[m.feedback as usize] * (self.modu[c].out1 + self.modu[c].out2) * 0.5;
            let m_out = wave(m.wave, self.modu[c].phase + fb) * lin(m_att);
            self.modu[c].phase = (self.modu[c].phase + m_inc).fract();
            self.modu[c].out2 = self.modu[c].out1;
            self.modu[c].out1 = m_out;
            self.modu[c].eg_advance(
                db_step(eff(m.ar, m_rks)),
                db_step(eff(m.dr, m_rks)),
                db_step(eff(release_rate(m.rr, ch), m_rks)),
                m.sl as f64 * 3.0,
                m.egt,
            );

            // --- carrier operator (patch bytes 1,3,5,7) ---
            let cr = OpParams::decode(&p, true);
            let c_rks = if cr.ksr { ksn } else { ksn >> 2 };
            let c_inc = phase_inc(ch.fnum, ch.block, cr.mult, cr.vib, pm_sin);
            // Carrier level comes from the channel volume (3 dB/step), not a TL.
            let c_att = ch.vol as f64 * 3.0
                + ksl_db(cr.ksl, ch.block, ch.fnum)
                + self.car[c].env
                + cr.am_db(am_sin);
            let c_out = wave(cr.wave, self.car[c].phase + m_out * FM_DEPTH) * lin(c_att);
            self.car[c].phase = (self.car[c].phase + c_inc).fract();
            self.car[c].eg_advance(
                db_step(eff(cr.ar, c_rks)),
                db_step(eff(cr.dr, c_rks)),
                db_step(eff(release_rate(cr.rr, ch), c_rks)),
                cr.sl as f64 * 3.0,
                cr.egt,
            );

            mix += c_out;
        }
        self.out = if self.silence {
            0.0
        } else {
            (mix as f32) * GAIN
        };
    }
}

/// Decoded per-operator instrument parameters.
struct OpParams {
    am: bool,
    vib: bool,
    egt: bool, // sustaining (vs percussive) envelope
    ksr: bool,
    mult: f64,
    ksl: u8,
    tl_db: f64, // modulator total level (carriers ignore this)
    ar: u8,
    dr: u8,
    sl: u8,
    rr: u8,
    wave: u8,
    feedback: u8,
}

impl OpParams {
    /// `carrier` picks the carrier bytes (1,3,5,7) over the modulator (0,2,4,6).
    fn decode(p: &[u8; 8], carrier: bool) -> Self {
        let b0 = p[carrier as usize]; // byte 0 or 1
        let b1 = p[2 + carrier as usize]; // byte 2 or 3
        let b2 = p[4 + carrier as usize]; // byte 4 or 5
        let b3 = p[6 + carrier as usize]; // byte 6 or 7
        OpParams {
            am: b0 & 0x80 != 0,
            vib: b0 & 0x40 != 0,
            egt: b0 & 0x20 != 0,
            ksr: b0 & 0x10 != 0,
            mult: MULT[(b0 & 0x0F) as usize],
            ksl: (b1 >> 6) & 3,
            tl_db: (p[2] & 0x3F) as f64 * 0.75, // always the modulator byte's TL
            ar: b2 >> 4,
            dr: b2 & 0x0F,
            sl: b3 >> 4,
            rr: b3 & 0x0F,
            // Waveform: carrier in byte3 bit4, modulator in byte3 bit3.
            wave: if carrier {
                (p[3] >> 4) & 1
            } else {
                (p[3] >> 3) & 1
            },
            feedback: p[3] & 7, // only meaningful for the modulator
        }
    }

    /// Tremolo attenuation contribution (≈1 dB depth) when AM is enabled.
    fn am_db(&self, am_sin: f64) -> f64 {
        if self.am { 0.5 * (1.0 + am_sin) } else { 0.0 }
    }
}

/// Phase increment per 49716 Hz sample, in cycles, with vibrato applied.
fn phase_inc(fnum: u16, block: u8, mult: f64, vib: bool, pm_sin: f64) -> f64 {
    // F = 49716 * fnum * 2^block / 2^19 (Hz); inc = F / 49716 (cycles/sample).
    let pm = if vib { 1.0 + 0.004 * pm_sin } else { 1.0 };
    fnum as f64 * (1u32 << block) as f64 * mult * pm / 524288.0
}

/// Effective envelope rate: 4*rate + key-scale, clamped 0..63. Rate 0 freezes.
fn eff(rate: u8, rks: i32) -> i32 {
    if rate == 0 {
        0
    } else {
        (4 * rate as i32 + rks).clamp(0, 63)
    }
}

/// The $2X sustain bit slows the key-off release to a fixed rate; with the key
/// still held (incl. percussive decay) the patch's own release rate applies.
fn release_rate(rr: u8, ch: Channel) -> u8 {
    if !ch.key && ch.sustain { 5 } else { rr }
}

/// dB of envelope movement per sample for an effective rate. Doubles every 4
/// rate steps, so the fastest rates are near-instant and the slowest decays
/// span seconds. The base is tuned so mid attack rates land in tens of ms.
fn db_step(eff: i32) -> f64 {
    if eff <= 0 {
        0.0
    } else {
        (0.00015 * 2f64.powf(eff as f64 / 4.0)).min(24.0)
    }
}

/// Key-scale level: extra attenuation (dB) for higher notes. Standard OPL
/// table in 0.375 dB units, weighted 1/8..1/2 by the KSL setting (1-3).
fn ksl_db(ksl: u8, block: u8, fnum: u16) -> f64 {
    const TABLE: [i32; 16] = [
        0, 32, 40, 45, 48, 51, 53, 55, 56, 58, 59, 60, 61, 62, 63, 64,
    ];
    if ksl == 0 {
        return 0.0;
    }
    let t = TABLE[((fnum >> 5) & 15) as usize] - 16 * (7 - block as i32);
    if t <= 0 {
        return 0.0;
    }
    (t >> (3 - ksl as i32)) as f64 * 0.375
}

/// Operator waveform: full sine (0) or half-wave-rectified sine (1, negative
/// half clipped to zero). `sin` is periodic, so an out-of-range phase from FM
/// or feedback is fine.
fn wave(w: u8, phase: f64) -> f64 {
    let s = (phase * std::f64::consts::TAU).sin();
    if w != 0 && s < 0.0 { 0.0 } else { s }
}

/// Convert an attenuation in dB to a linear amplitude (0..1).
fn lin(att_db: f64) -> f64 {
    if att_db >= MAX_ATT {
        0.0
    } else {
        10f64.powf(-att_db / 20.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vrc7() -> Vrc7 {
        // 16 x 8KB PRG, 32 x 1KB CHR; each byte = its bank index.
        let prg: Vec<u8> = (0..16 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..32 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Vrc7::new(2, prg, chr, Mirroring::Vertical) // VRC7a ($x010 select)
    }

    /// Write one OPLL register via the address/data ports (VRC7a addresses).
    fn opll(m: &mut Vrc7, reg: u8, val: u8) {
        m.cpu_write(0x9010, reg);
        m.cpu_write(0x9030, val);
    }

    #[test]
    fn prg_banking() {
        let mut m = vrc7();
        m.cpu_write(0x8000, 1); // $8000 bank
        m.cpu_write(0x8010, 2); // $A000 bank (A4 select)
        m.cpu_write(0x9000, 3); // $C000 bank
        assert_eq!(m.cpu_read(0x8000), 1);
        assert_eq!(m.cpu_read(0xA000), 2);
        assert_eq!(m.cpu_read(0xC000), 3);
        assert_eq!(m.cpu_read(0xE000), 15); // fixed last 8KB bank
    }

    #[test]
    fn chr_banking() {
        let mut m = vrc7();
        m.cpu_write(0xA000, 5); // $0000-$03FF
        m.cpu_write(0xD010, 9); // $1C00-$1FFF (A4 select -> bank 7)
        assert_eq!(m.ppu_read(0x0000), 5);
        assert_eq!(m.ppu_read(0x1C00), 9);
    }

    #[test]
    fn chr_ram_rw_when_no_chr_rom() {
        // Lagrange Point ships no CHR ROM; the board carries 8KB CHR RAM.
        let prg: Vec<u8> = (0..16 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let mut m = Vrc7::new(2, prg, Vec::new(), Mirroring::Vertical);
        m.cpu_write(0xA000, 0); // map RAM bank 0 at $0000
        m.ppu_write(0x0123, 0xAB);
        assert_eq!(m.ppu_read(0x0123), 0xAB);
    }

    #[test]
    fn mirroring_and_prg_ram() {
        let mut m = vrc7();
        m.cpu_write(0xE000, 1); // horizontal ($E000, A4=0)
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0x6000, 0xAB);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAB));
    }

    #[test]
    fn irq_cycle_mode() {
        let mut m = vrc7();
        m.cpu_write(0xE010, 0xFD); // latch via $E010 (A4 select)
        m.cpu_write(0xF000, 0x06); // enable, cycle mode
        for _ in 0..2 {
            m.cpu_clock();
            assert!(!m.irq());
        }
        m.cpu_clock();
        assert!(m.irq());
    }

    #[test]
    fn fm_audio_keys_on_and_decays() {
        let mut m = vrc7();
        opll(&mut m, 0x30, 0x10); // channel 0: instrument 1, volume 0 (loudest)
        opll(&mut m, 0x10, 0x40); // F-number low
        opll(&mut m, 0x20, 0x14); // block 2, key on
        let mut peak = 0.0f32;
        for _ in 0..150_000 {
            m.cpu_clock();
            peak = peak.max(m.audio_sample().abs());
        }
        assert!(peak > 0.02, "expected audible VRC7 FM output, got {peak}");

        // Key off and let the release run to completion, then confirm the tail
        // has died away to (near) silence.
        opll(&mut m, 0x20, 0x04); // key off (block 2, key bit clear)
        for _ in 0..400_000 {
            m.cpu_clock();
        }
        let mut tail = 0.0f32;
        for _ in 0..5_000 {
            m.cpu_clock();
            tail = tail.max(m.audio_sample().abs());
        }
        assert!(
            tail < 0.005,
            "release did not decay to silence: tail {tail}"
        );
    }

    #[test]
    fn silence_bit_mutes() {
        let mut m = vrc7();
        opll(&mut m, 0x30, 0x10);
        opll(&mut m, 0x10, 0x40);
        opll(&mut m, 0x20, 0x14);
        for _ in 0..20_000 {
            m.cpu_clock();
        }
        m.cpu_write(0xE000, 0x80); // silence expansion sound
        for _ in 0..100 {
            m.cpu_clock();
        }
        assert_eq!(m.audio_sample(), 0.0);
    }
}
