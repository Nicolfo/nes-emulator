# NES Emulator

A NES (Nintendo Entertainment System) emulator written in Rust, targeting mapper 0
(NROM) games such as Super Mario Bros. Video, audio and input.

## Running

```
cargo run --release                  # opens the home menu
cargo run --release -- path\to\rom.nes   # skips the menu, boots the ROM directly
```

The home menu offers **Load ROM** (native file picker), **Settings**, **Resume**
(when a game is loaded) and **Quit**. Settings lets you rebind every controller
button (select a row, press Enter, then press the new key) and change the window
scale; everything is persisted to `nes-emulator-config.json`.

## Default controls

| NES button | Key |
|---|---|
| D-Pad | Arrow keys |
| A | Z |
| B | X |
| Start | Enter |
| Select | Right Shift |
| Back to menu | Escape |

All bindings except Escape can be changed in Settings.

## Architecture

- `src/cpu.rs` — 6502 core: all official opcodes plus the unofficial NOP family,
  instruction-level cycle counting with page-cross/branch penalties, NMI and IRQ
  handling (the APU frame counter and DMC raise IRQs), hardware quirks (JMP
  indirect page-wrap bug, zero-page pointer wrap, B-flag rules).
- `src/ppu.rs` — dot-stepped PPU: loopy v/t/x/w scroll registers, per-dot background
  pipeline with shift registers, scanline-batched sprite evaluation, exact-pixel
  sprite 0 hit (needed for SMB's status-bar scroll split), buffered $2007 reads,
  palette mirroring.
- `src/apu.rs` — NTSC APU, ticked once per CPU cycle: pulse 1/2 (duty sequencer,
  envelope, sweep with pulse 1's ones'-complement negate, continuous mute logic),
  triangle (linear counter, DAC holds its value when halted), noise (15-bit LFSR,
  both tap modes), DMC (real memory fetches with 4-cycle CPU stall, $8000 address
  wrap, loop, IRQ). Frame counter in exact CPU-cycle timing: 4- and 5-step modes,
  the 3-cycle IRQ flag window at the end of mode 0, the 3/4-cycle $4017 write
  delay, IRQ inhibit. Non-linear mixer via the nesdev lookup-table formulas, then
  boxcar decimation to the host sample rate followed by the NES analog filter
  chain (90 Hz + 440 Hz high-pass, 14 kHz low-pass).
- `src/bus.rs` — CPU memory map, OAM DMA with 513/514-cycle stall, DMC sample DMA,
  APU register routing ($4000–$4013, $4015, $4017), 3 PPU dots and 1 APU step per
  CPU cycle interleave, NMI edge and level-triggered IRQ propagation.
- `src/mapper.rs` — `Mapper` trait + NROM implementation (extension point for more mappers).
- `src/cartridge.rs` — iNES header parsing.
- `src/controller.rs` — standard joypad strobe/shift register.
- `src/main.rs` — winit 0.30 + pixels frontend, home/settings/running state machine,
  paced at the NTSC rate of 60.0988 FPS while a game runs. Dynamic audio rate
  control nudges the resampling ratio (±0.3 %) so the audio queue hovers around
  50 ms instead of drifting into under/overflow.
- `src/audio.rs` — cpal output stream (f32/i16/u16 device formats) fed from a
  shared sample queue; underruns decay to silence to avoid clicks.
- `src/menu.rs`, `src/font.rs` — NES-style menu UI rendered into the same 256x240
  framebuffer (embedded 8x8 bitmap font, pixel-art icons).
- `src/config.rs` — persisted settings (key bindings, window scale).

Timing: NTSC, 1 CPU cycle = 3 PPU dots = 1 APU step, interleaved at instruction
granularity (89,342 dots/frame). The odd-frame dot skip and the $2002
NMI-suppression race are intentionally not implemented; they don't affect
mapper-0 era games.

## Tests

```
cargo test
```

- `tests/nestest.rs` — CPU validated against the nestest golden log (registers and
  cycle counts for all official opcodes + unofficial NOPs, log lines 1–5259).
  Requires `tests/data/nestest.nes` and `tests/data/nestest.log` (skipped if absent).
- `tests/smb.rs` — headless SMB smoke tests (title screen renders, gameplay
  reachable, gameplay music produces non-clipping audio — the title screen itself
  is silent). The ignored `dump_frame_bmp` test writes `frame.bmp` for visual
  inspection: `cargo test --test smb -- --ignored` with env vars `SMB_FRAMES`,
  `SMB_PRESS_START`, `SMB_RUN_RIGHT`.
- Unit tests cover loopy scroll register sequences, palette mirroring, $2007 read
  buffering, controller shifting, RAM mirroring, OAM DMA, and the APU (frame IRQ
  timing and inhibit, length counter load/countdown, sweep muting, DMC fetch and
  IRQ, audible pulse output).

ROMs are not included except for any you place in the project directory.
