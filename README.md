# NES Emulator

A NES (Nintendo Entertainment System) emulator written in Rust, targeting mapper 0
(NROM) games such as Super Mario Bros. Video and input only — no audio.

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
  instruction-level cycle counting with page-cross/branch penalties, NMI handling,
  hardware quirks (JMP indirect page-wrap bug, zero-page pointer wrap, B-flag rules).
- `src/ppu.rs` — dot-stepped PPU: loopy v/t/x/w scroll registers, per-dot background
  pipeline with shift registers, scanline-batched sprite evaluation, exact-pixel
  sprite 0 hit (needed for SMB's status-bar scroll split), buffered $2007 reads,
  palette mirroring.
- `src/bus.rs` — CPU memory map, OAM DMA with 513/514-cycle stall, 3 PPU dots per
  CPU cycle interleave, NMI edge propagation.
- `src/mapper.rs` — `Mapper` trait + NROM implementation (extension point for more mappers).
- `src/cartridge.rs` — iNES header parsing.
- `src/controller.rs` — standard joypad strobe/shift register.
- `src/main.rs` — winit 0.30 + pixels frontend, home/settings/running state machine,
  paced at the NTSC rate of 60.0988 FPS while a game runs.
- `src/menu.rs`, `src/font.rs` — NES-style menu UI rendered into the same 256x240
  framebuffer (embedded 8x8 bitmap font, pixel-art icons).
- `src/config.rs` — persisted settings (key bindings, window scale).

Timing: NTSC, 1 CPU cycle = 3 PPU dots, interleaved at instruction granularity
(89,342 dots/frame). The odd-frame dot skip and the $2002 NMI-suppression race are
intentionally not implemented; they don't affect mapper-0 era games.

## Tests

```
cargo test
```

- `tests/nestest.rs` — CPU validated against the nestest golden log (registers and
  cycle counts for all official opcodes + unofficial NOPs, log lines 1–5259).
  Requires `tests/data/nestest.nes` and `tests/data/nestest.log` (skipped if absent).
- `tests/smb.rs` — headless SMB smoke tests (title screen renders, gameplay reachable).
  The ignored `dump_frame_bmp` test writes `frame.bmp` for visual inspection:
  `cargo test --test smb -- --ignored` with env vars `SMB_FRAMES`, `SMB_PRESS_START`,
  `SMB_RUN_RIGHT`.
- Unit tests cover loopy scroll register sequences, palette mirroring, $2007 read
  buffering, controller shifting, RAM mirroring, and OAM DMA.

ROMs are not included except for any you place in the project directory.
