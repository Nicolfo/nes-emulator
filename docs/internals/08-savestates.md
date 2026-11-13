# 7 - Savestates

A **savestate** freezes the entire emulated machine to a blob of bytes and
thaws it later, restoring play to the exact instant it was taken - mid-frame,
mid-instruction, mid-DMA. This is different from a **battery save** (chapter 5),
which only persists the cartridge's PRG RAM across sessions the way a real
save-game does. A savestate is the whole console, not just the game's save slot.

The frontend exposes **four** per-ROM slots through a picker UI: **F5** pauses
the game and opens the save picker, **F7** the load picker; the chosen slot maps
to `<rom>.stateN` (N = 1..4) next to the ROM (see
[chapter 6](07-frontend.md)). The library API is two methods on the `Nes`
facade:

```rust
fn save_state(&self) -> Result<Vec<u8>, String>;
fn load_state(&mut self, data: &[u8]) -> Result<(), String>;
```

## What a snapshot has to contain

The machine's state is spread across every chip, and *all* of the mutable parts
must be captured or the restore won't be bit-exact:

- **CPU** - registers (`A`/`X`/`Y`/`SP`/`PC`/`P`), the cycle counter, and the
  interrupt-pipeline flags (NMI edge latch, the two-stage IRQ poll, the
  BRK/interrupt poll-suppression and OAM-DMA bookkeeping). Skipping the pipeline
  flags would mis-time an interrupt that was one cycle from firing.
- **Bus** - the 2 KB work RAM, the cycle counter, the PAL dot-phase counter,
  the open-bus / internal-bus latches, the pending OAM/DMC DMA latches, and
  on Vs. System boards the protection-chip / Vs. Zapper / DualSystem latches
  (`vs_latch`).
- **PPU** - the loopy `v`/`t`/`x`/`w` scroll state, `CTRL`/`MASK`/`STATUS`, OAM
  and secondary OAM, the 2 KB CIRAM, palette RAM, the background and sprite
  pipeline shifters/latches, and the scanline/dot position.
- **APU** - all five channels (pulse ×2, triangle, noise, DMC) and the frame
  counter.
- **Controller** - the strobe line and shift register.
- **Cartridge / mapper** - banking registers, IRQ counters, PRG/CHR RAM, and any
  expansion-audio state (but not PRG or CHR ROM - see "The mapper boundary").
- **Vs. DualSystem** - the second board's complete state, nested as
  `vs_sub` (a whole `SaveState`); a state with or without it only loads into
  the matching setup.

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

- **`byte_array`** - a `serde` `with`-module for fixed `[u8; N]` arrays larger
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

The PRG ROM is marked `#[serde(skip)]` in every mapper and grafted back from
the live instance on load - the ROM is immutable and already loaded, and
serializing up to a megabyte of it as JSON numbers dominated the state size.
CHR ROM is left out too, without each board having to say whether its CHR is
ROM or RAM: mappers hold CHR in a [`Chr`](../../src/mapper/chr.rs), a
copy-on-write buffer over the power-on image that marks itself dirty on the
first mutable access. An untouched `Chr` (CHR ROM, or CHR RAM nothing has
written yet) serializes as `null`; a written one serializes in full exactly
like the old `Vec<u8>`, so CHR RAM is preserved. On load the macro restores
each listed field through `StateField::restore`: a `null` CHR gets the live
instance's power-on image grafted back, real data must match the live size.
States written before this (with CHR ROM as a full array) still load; if the
array equals the ROM the buffer goes clean again, so re-saving drops it. For
SMB3 this took a snapshot from 452 KB to 37 KB and `Nes::snapshot` from
1.7 ms to 0.15 ms, which is what rewind capture pays every other frame.

The same `StateField::restore` pass length-guards the resizable buffers (CHR,
PRG RAM, CHR RAM) so a mismatched or hand-edited state is rejected instead of
panicking later bank math.

## Format versions

- **v1** stored the mapper's JSON state as a JSON *array of its bytes*
  (`"mapper":[123,34,...]`), three to four times its own size.
- **v2** (current) embeds the mapper state as a JSON value via the
  `savestate::mapper_json` serde helper. Loading still accepts v1: an array
  is read back as bytes, anything else is re-serialized to the mapper's JSON.

## Validation on load

A savestate is only meaningful for the ROM it came from. `load_state` guards
against obvious mismatches before applying anything:

- a 32-bit **magic** number and a format **version** (bump `VERSION` in
  `savestate.rs` on any incompatible layout change; `MIN_VERSION` is the
  oldest still accepted), and
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
