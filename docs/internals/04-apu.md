# 3 - The APU (audio processing unit)

## The hardware

The APU lives inside the same 2A03 chip as the CPU and is clocked at the CPU
rate. It has **five sound channels**, each a simple digital generator, summed
through a deliberately **non-linear** mixer to an analog output pin.

### The five channels

| Channel | Sound | Key parameters |
|---------|-------|----------------|
| **Pulse 1** | Square wave | Duty cycle (12.5/25/50/75%), volume/envelope, sweep, length |
| **Pulse 2** | Square wave | Same as pulse 1, with a slightly different sweep |
| **Triangle** | Triangle wave | Stepped 0→15→0 sequence; linear + length counters |
| **Noise** | Pseudo-random | 15-bit LFSR, two tap modes (tonal vs. hiss), volume/envelope, length |
| **DMC** | Sample playback | Plays 1-bit delta-modulated samples fetched from PRG memory |

Each channel has a **timer** (a down-counter reloaded from a period value) that
sets its frequency, plus shared support units:

- **Length counter** - auto-silences a channel after a programmed duration unless
  "halted". Loaded from a 32-entry lookup table.
- **Envelope** (pulse + noise) - either a constant volume or a decaying ramp.
- **Sweep** (pulse only) - slides the pitch up or down over time; also has
  "muting" logic that silences out-of-range periods even when disabled.
- **Linear counter** (triangle only) - a finer-grained gate than the length
  counter.

### The frame counter

A separate **frame counter / frame sequencer** clocks the support units on a
fixed schedule, roughly four times per video frame. It has two modes:

- **4-step mode** - clocks the units in a 4-step cycle and, at the end of the
  cycle, can raise a **frame IRQ** (unless inhibited).
- **5-step mode** - a longer cycle with no IRQ.

The frame counter is driven by the CPU clock, and its IRQ timing is precise
enough that games and test ROMs depend on the exact cycles involved.

### The DMC and its DMA

The DMC (delta modulation channel) is special: it plays back compressed audio
samples that live in the cartridge's PRG ROM. To get each sample byte, the DMC
**steals a CPU cycle** to perform a memory read (a DMA). This stall is observable
- it can delay the CPU by a few cycles at a time - and interacts with the OAM
DMA and with controller reads in ways real games stumbled over. The mechanics of
that stall are a *CPU/bus* concern and are covered in
[chapter 4](05-bus-timing-dma.md); this chapter covers the DMC as a *sound
generator*.

### The non-linear mixer

The channels do **not** sum linearly. The hardware mixes the two pulses through
one non-linear function and the triangle/noise/DMC through another. The standard
emulation approach (which this uses) is two lookup tables derived from the
documented formulas. The analog output is then shaped by a fixed filter chain: a
high-pass at ~90 Hz, another at ~440 Hz, and a low-pass at ~14 kHz.

## The implementation

The APU ([`src/apu.rs`](../../src/apu.rs)) is ticked once per CPU cycle by the
bus ([`Apu::tick`](../../src/apu.rs)). Each channel is its own small struct with
a `clock_timer`, an `output`, and the relevant support methods.

### Channel structs

- [`Pulse`](../../src/apu.rs) - duty sequencer (`DUTY` table), `Envelope`, and a
  full sweep unit (`sweep_target`, `muted`, `clock_sweep`). The
  `ones_complement` flag captures the one-bit difference between pulse 1 and
  pulse 2's negate behavior. `output` returns 0 when muted/silenced, else the
  envelope volume.
- [`Triangle`](../../src/apu.rs) - steps through the 32-entry `TRIANGLE_SEQ`;
  gated by both the length and linear counters. Note `output` keeps returning the
  current step when halted (the DAC holds its value - no click).
- [`Noise`](../../src/apu.rs) - a 15-bit LFSR (`shift`) with the two tap modes;
  period comes from a region-specific table (`NOISE_PERIOD` / `PAL_NOISE_PERIOD`).
- [`Dmc`](../../src/apu.rs) - the delta decoder: `clock_output` shifts the
  current byte one bit and nudges `level` ±2, and when the bit buffer empties it
  pulls the next byte (which the DMA delivered via `supply`). It tracks
  `bytes_remaining`, address wrap (`$FFFF`→`$8000`), looping, and the DMC IRQ.

`Envelope` ([`src/apu.rs`](../../src/apu.rs)) is shared by the pulses and noise.

### Timer clocking rates

A subtle but important detail handled in `tick`: the channels clock at different
rates.

- Pulse timers clock every **other** CPU cycle (APU cycle): `if self.cycle & 1 == 0`.
- Triangle, noise, and DMC timers clock **every** CPU cycle.

This is why the triangle can reach higher frequencies than the pulses.

### The frame counter

[`clock_frame_events`](../../src/apu.rs) (and `clock_frame_events_pal`) implement
the sequencer in **exact CPU-cycle counts**, not approximate divisions. In
4-step NTSC mode the quarter-frame clocks (`clock_quarter`: envelopes + triangle
linear counter) and half-frame clocks (`clock_half`: length counters + sweeps)
land at cycles 7457, 14913, 22371, and 29829, and the frame IRQ flag is driven
across the **3-cycle window** 29828–29830.

> **Quirk - the frame IRQ window.** Even with the IRQ-inhibit bit set, the first
> two cycles of that window still set the *readable* flag; only the final cycle
> respects inhibit. [`set_frame_irq`](../../src/apu.rs) encodes this. Also, the
> `$4017` write that selects the mode takes effect 3 or 4 cycles later depending
> on the cycle parity (`frame_reset_delay`), and writing mode 5 immediately
> issues one quarter + half clock.

> **Quirk - $4015 read clears the frame IRQ, but late.** Reading the status
> register clears the frame IRQ flag - but the clear only lands on the next
> "get" cycle (`frame_irq_clear_pending`), not instantly.

### Register writes

[`Apu::write`](../../src/apu.rs) decodes `$4000`–`$4017`. Each channel's four
registers set duty/volume, sweep, timer-low, and timer-high+length (the
timer-high write also resets phase and restarts the envelope, mirroring
hardware). `$4015` is the enable/status register: writing it enables/disables
channels (disabling zeroes the length counter) and arms the DMC; `$4017`
configures the frame counter.

The DMC enable path in the `$4015` handler is where most of the
[chapter 4](05-bus-timing-dma.md) DMA timing originates (`load_dma`,
`pending_disable`, `enable_delay`, etc.) - those fields decide *when* a sample
fetch DMA is raised and whether it is a normal, "ghost" (aborted), or
"blocked-retry" DMA. As a sound generator you can mostly ignore them; they exist
to get the CPU-stall timing right.

### The mixer and resampling

[`mix`](../../src/apu.rs) implements the non-linear mix via two precomputed
lookup tables built in `Apu::new`:

```rust
pulse_table[n]  = 95.52  / (8128.0  / n + 100.0);   // indexed by pulse1+pulse2
tnd_table[n]    = 163.67 / (24329.0 / n + 100.0);   // indexed by 3*tri + 2*noise + dmc
```

The APU runs at ~1.79 MHz but the host sound card wants ~48 kHz, so `tick` does
**boxcar decimation**: it accumulates every cycle's mixed sample into `acc` and,
once enough CPU cycles have elapsed per output sample (`cycles_per_sample`),
averages them into one output sample and passes it through the analog filter
chain - `hp1` (90 Hz), `hp2` (440 Hz), `lp` (14 kHz) - before pushing it to the
`samples` buffer. [`HighPass`](../../src/apu.rs) and [`LowPass`](../../src/apu.rs)
are textbook one-pole RC filters.

The cartridge's optional **expansion audio** (some mappers add their own sound
channels, from the VRC6/Sunsoft 5B square waves up to VRC7's six-channel OPLL FM
synth) is summed in here too: `tick` takes an `ext` argument - the mapper's
`audio_sample()` - and adds it before decimation, so expansion sound rides the
same filters.

> **Region differences.** PAL uses a different CPU clock (`PAL_CPU_HZ`),
> different noise/DMC period tables, and different frame-counter cycle counts.
> `set_region` and the `*_pal` methods switch these; the resampling ratio is
> recomputed so audio stays pitched correctly.

### Pulling audio out

The host drains generated samples with [`Apu::take_samples`](../../src/apu.rs)
(exposed up the stack as `Nes::take_audio`). Dynamic rate control on the frontend
nudges `cycles_per_sample` slightly via [`Apu::tune`](../../src/apu.rs) to keep
the host audio queue from drifting into under/overflow - see
[chapter 6](07-frontend.md).

### Where to look

| You want to understand… | Look at |
|---|---|
| One channel | `Pulse`, `Triangle`, `Noise`, `Dmc`, `Envelope` |
| Timer rates | `Apu::tick` (the `cycle & 1` split) |
| Frame sequencer + IRQ | `clock_frame_events`, `clock_quarter`, `clock_half`, `set_frame_irq` |
| Register decode | `Apu::write`, `read_status` |
| Mixing + filtering + resampling | `mix`, `Apu::tick` tail, `HighPass`, `LowPass` |
