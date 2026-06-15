# 7 — Savestates

A **savestate** freezes the entire emulated machine to a blob of bytes and
thaws it later, restoring play to the exact instant it was taken — mid-frame,
mid-instruction, mid-DMA. This is different from a **battery save** (chapter 5),
which only persists the cartridge's PRG RAM across sessions the way a real
save-game does. A savestate is the whole console, not just the game's save slot.

The frontend exposes it as a single per-ROM slot: **F5** writes `<rom>.state`,
**F7** reads it back (see [chapter 6](07-frontend.md)). The library API is two
methods on the `Nes` facade:

```rust
fn save_state(&self) -> Result<Vec<u8>, String>;
fn load_state(&mut self, data: &[u8]) -> Result<(), String>;
```

## What a snapshot has to contain

The machine's state is spread across every chip, and *all* of the mutable parts
must be captured or the restore won't be bit-exact:

- **CPU** — registers (`A`/`X`/`Y`/`SP`/`PC`/`P`), the cycle counter, and the
  interrupt-pipeline flags (NMI edge latch, the two-stage IRQ poll, the
  BRK/interrupt poll-suppression and OAM-DMA bookkeeping). Skipping the pipeline
  flags would mis-time an interrupt that was one cycle from firing.
- **Bus** — the 2 KB work RAM, the cycle counter, the PAL dot-phase counter,
  the open-bus / internal-bus latches, and the pending OAM/DMC DMA latches.
- **PPU** — the loopy `v`/`t`/`x`/`w` scroll state, `CTRL`/`MASK`/`STATUS`, OAM
  and secondary OAM, the 2 KB CIRAM, palette RAM, the background and sprite
  pipeline shifters/latches, and the scanline/dot position.
- **APU** — all five channels (pulse ×2, triangle, noise, DMC) and the frame
  counter.
- **Controller** — the strobe line and shift register.
- **Cartridge / mapper** — banking registers, IRQ counters, PRG/CHR RAM, and any
  expansion-audio state.

## What is deliberately left out

Two pieces of state are *host* concerns, not console state, and are excluded so
a state saved on one machine restores cleanly on another:

- **The PPU framebuffer.** It's pure output, fully regenerated within one frame,
  so it's marked `#[serde(skip)]` (and defaults back to a correctly-sized black
  buffer on load). This keeps the blob ~250 KB smaller.
- **The APU's resampling and filter chain.** The decimation accumulator, the
  high/low-pass filter coefficients, and the output sample queue are all tied to
  the *host* audio output rate, which the frontend configures at startup via
  `set_sample_rate`. `Apu::save_state` snapshots only the channel and
  frame-counter state into an `ApuSave` struct; `Apu::load_state` writes those
  back into the live APU, leaving its host audio configuration untouched.

## How it's serialized

The format is `serde_json`. `serde` is already a dependency (used for the config
file), and JSON keeps the format debuggable. The leaf state structs derive
`Serialize`/`Deserialize` directly; the orchestration lives in
[`src/savestate.rs`](../../src/savestate.rs), which defines the top-level
[`SaveState`](../../src/savestate.rs) aggregate plus two small pieces of glue:

- **`byte_array`** — a `serde` `with`-module for fixed `[u8; N]` arrays larger
  than 32 elements. `serde` only derives array impls up to length 32, so the
  work RAM, OAM, CIRAM, ExRAM, and the mappers' PRG RAM ride through this helper
  (serialized as a byte sequence, deserialized back into the fixed array).
- **DTOs for the CPU, bus, and APU** (`CpuSave`, `BusSave`, `ApuSave`). These
  three can't simply derive `Serialize` on the live struct: the `Bus` owns the
  `Box<dyn Mapper>` trait object and the host-only debug watch logs, the `Cpu`
  owns the `Bus`, and the `Apu` owns the host audio chain. The DTOs name exactly
  the fields worth persisting.

The PPU and controller, having no such entanglements, are snapshotted by
`#[derive]`-ing `Serialize`/`Deserialize` on the structs themselves.

### The mapper boundary

`Box<dyn Mapper>` is the awkward one: you can't deserialize a trait object
without knowing its concrete type. Rather than a type registry, the `Mapper`
trait carries its own serialization across the dynamic boundary:

```rust
fn save_state(&self) -> Vec<u8>;
fn load_state(&mut self, data: &[u8]) -> Result<(), String>;
```

Each mapper derives `Serialize`/`Deserialize` and implements these two methods
via the [`impl_mapper_savestate!`](../../src/mapper.rs) macro, which just
round-trips the whole struct through `serde_json`. On restore, the already-built
mapper (it knows its concrete type) deserializes the bytes into itself. A blob
from a different mapper simply fails to deserialize, which is reported as an
error rather than silently corrupting state.

This means the mapper blob includes the cartridge's PRG/CHR ROM. That's
redundant (the ROM is immutable and already loaded), but it keeps the per-mapper
code to a single macro line and makes the blob fully self-describing; the size
cost is acceptable for a manually triggered feature.

## Validation on load

A savestate is only meaningful for the ROM it came from. `load_state` guards
against obvious mismatches before applying anything:

- a 32-bit **magic** number and a format **version** (bump `VERSION` in
  `savestate.rs` on any incompatible layout change), and
- the **TV region** must match the loaded ROM.

A state from the wrong game still usually fails at the mapper-deserialize step
(different field shapes), and the per-ROM `<rom>.state` filename keeps the
frontend from offering one game's state to another in the first place.

## Where to look

| You want to understand… | Look at |
|---|---|
| The blob format + `byte_array` helper | `src/savestate.rs` |
| The public API + load-time validation | `Nes::save_state`/`load_state` (`src/nes.rs`) |
| CPU / bus / APU snapshot DTOs | `CpuSave`, `BusSave`, `ApuSave` |
| The mapper serialization boundary | `Mapper::save_state`/`load_state`, `impl_mapper_savestate!` (`src/mapper.rs`) |
| F5/F7 wiring | `App::save_state`/`load_state` (`src/main.rs`) |
