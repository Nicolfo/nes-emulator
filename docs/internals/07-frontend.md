# 6 - The frontend

Everything so far has been the emulated *console*. This chapter covers the parts
that connect it to your actual computer: the console facade, the windowing/input
host, audio output, and the controller. These live outside the library's
emulation core (the host code is in `src/main.rs` and friends, which are *not*
part of `lib.rs`).

## The `Nes` facade

[`src/nes.rs`](../../src/nes.rs) is the seam between "the console" and "the
program running it." It wraps a `Cpu` (which owns the `Bus`, which owns
everything else) and exposes a small, frontend-agnostic API:

| Method | Purpose |
|--------|---------|
| `Nes::new(rom)` | Parse the ROM, build the console, reset the CPU |
| `run_frame()` | Step the CPU until the PPU finishes a frame |
| `reset()` | Soft reset: pulse the PPU/APU reset lines + re-run CPU reset, keeping RAM/VRAM |
| `framebuffer()` | Borrow the 256×240 RGBA pixel buffer |
| `region()` | NTSC or PAL (drives frame pacing) |
| `set_sample_rate` / `tune_audio` / `take_audio` | Audio plumbing |
| `battery_ram` / `load_battery_ram` | Save-game persistence |
| `save_state` / `load_state` | Whole-machine snapshots (see [chapter 7](08-savestates.md)) |

This is the *only* surface the GUI and the test harnesses need; both drive the
emulator purely through it. `run_frame` is the loop described in
[chapter 0](01-architecture.md): clear `frame_complete`, step the CPU until the
PPU sets it again at vblank.

## The controller

[`src/controller.rs`](../../src/controller.rs) models the standard NES joypad,
which is a humble **shift register**:

- Eight buttons live in a single byte `state` (A, B, Select, Start, Up, Down,
  Left, Right - the `BTN_*` constants).
- Writing `$4016` with bit 0 set raises the **strobe**: while high, the pad
  continuously reloads its shift register from the live button state.
- When strobe drops, each read returns the next button bit (A first) and shifts;
  after the 8 buttons it returns 1s.

The bus owns **two** of these - `controller1` (read at `$4016`) and
`controller2` (read at `$4017`) - for the two physical joypad ports. A `$4016`
write strobes both at once; reads come back on the matching port. The host wires
keyboard input to both: player 1 from `cfg.keys`, player 2 from `cfg.keys_p2`.

The host doesn't call the shift register directly during gameplay - it just sets
button state via `set_button` (from key events), and the *bus* clocks the
strobe/shift at the right cycle parity (`clock_put_cycle`, called from
`tick_cycle_post`; see [chapter 4](05-bus-timing-dma.md)). The
reload-on-put-cycle detail is what makes the shifting cycle-accurate.

## The host application (`src/main.rs`)

The frontend is a [winit](https://docs.rs/winit) 0.30 + [pixels](https://docs.rs/pixels)
application - a small state machine with four views:

```rust
enum View { Home { .. }, Settings { .. }, SlotPicker { .. }, Running }
```

- **Home / Settings** render the NES-style menu UI (drawn into the same 256×240
  framebuffer by `src/menu.rs` using the embedded bitmap font in `src/font.rs`).
  Settings let you rebind keys per player (an EDIT PLAYER toggle switches the
  button rows between player 1 and player 2), change window scale, toggle NTSC
  overscan cropping and reset defaults - all persisted via `src/config.rs`.
- **SlotPicker** is the savestate overlay shown over the paused game; its
  `saving` flag picks F5 (save) vs F7 (load) behaviour (see Savestates below).
- **Running** is where the emulation actually pumps.

### Frame pacing

The pacing lives in [`about_to_wait`](../../src/main.rs), winit's "idle" hook. It
targets the real console's exact refresh rate:

- NTSC: **60.0988 FPS** (`FRAME_PERIOD` ≈ 16.639 ms)
- PAL: **50.0070 FPS** (`PAL_FRAME_PERIOD` ≈ 19.997 ms)

Each idle pass, it runs as many frames as real time says are due (`while now >=
next_frame`), with a catch-up cap of 3 so a stall (e.g. dragging the window)
resyncs instead of spiraling. After running, it pushes audio and requests a
redraw. Redraw ([`WindowEvent::RedrawRequested`](../../src/main.rs)) copies the
emulator's framebuffer into the pixels surface, optionally cropping NTSC overscan
(`OVERSCAN_TOP`/`OVERSCAN_BOTTOM`).

### Audio output and dynamic rate control

[`src/audio.rs`](../../src/audio.rs) opens a [cpal](https://docs.rs/cpal) output
stream and feeds it from a shared sample queue; underruns decay to silence to
avoid clicks. The interesting bit is in `about_to_wait`: the emulator generates
audio at a rate tied to *emulated* time, but the sound card consumes at its own
real rate, and the two drift. So the frontend measures how full the audio queue
is and nudges the resampling ratio by up to ±0.3 % via `nes.tune_audio` to keep
the queue hovering around ~50 ms of buffered audio - neither starving nor
overflowing. This is why `Apu::tune` exists separately from `set_sample_rate`
(see [chapter 3](04-apu.md)).

### Battery saves

When a game with battery RAM is loaded, the host restores `<rom>.sav` into PRG
RAM (`restore_battery_ram`) and writes it back on exit / game switch / return to
menu (`save_battery_ram`), all through the `Nes::battery_ram` accessors from
[chapter 5](06-cartridge-mappers.md).

### Soft reset

While running, **F3** triggers a soft reset (the console's RESET button) through
`Nes::reset`: it pulses the PPU and APU reset lines and re-runs the CPU reset
sequence, so execution resumes from the cartridge's reset vector while PRG/CHR
RAM, VRAM, palette and OAM all survive - exactly what the hardware RESET line
does. `Cpu::reset` additionally drops any NMI/IRQ latched before the reset, so
the reset handler runs instead of being hijacked into an interrupt sequence. F3
is fixed (not rebindable), like the F5/F7 savestate keys.

### Savestates

While running, **F5** and **F7** pause the game and open the `SlotPicker`
overlay - F5 to save, F7 to load. It offers `menu::NUM_SLOTS` (4) slots stored
next to the ROM as `<rom>.stateN`; `App::slot_states` marks which are filled,
and confirming a slot calls `App::save_state` / `App::load_state`. Unlike a
battery save - which only persists PRG RAM between sessions - a savestate
captures the *exact* live state of every chip, so you can resume mid-frame. The
format and what is (and isn't) captured are covered in
[chapter 7](08-savestates.md).

## How a frame flows, end to end

Putting the whole guide together, one frame of gameplay is:

```
main.rs about_to_wait
  └─ nes.run_frame()                         (nes.rs)
       └─ loop: cpu.step()                   (cpu.rs)
            ├─ fetch opcode, execute it
            └─ each bus access = one cycle:
                 fetch_cycle
                   ├─ bus.tick_cycle_pre  → apu.tick ×1, ppu.tick ×2   (bus.rs)
                   ├─ bus.read/write      → RAM / PPU regs / APU / mapper
                   └─ bus.tick_cycle_post → ppu.tick ×1, controller strobe
            (PPU sets frame_complete at scanline 241, dot 1)
  ├─ nes.take_audio()  → audio queue + dynamic rate control
  └─ request redraw    → copy nes.framebuffer() to the window
```

Sixty times a second, that loop turns 30,000-ish CPU instructions and ~90,000
PPU dots into one image and a slice of sound - the same work the real silicon did
in 1985, just cycle by cycle in software.

### Where to look

| You want to understand… | Look at |
|---|---|
| The console API | `src/nes.rs` |
| The joypad | `src/controller.rs` |
| Window / input / view state machine | `src/main.rs` |
| Frame pacing + audio rate control | `about_to_wait` (main.rs) |
| Audio device output | `src/audio.rs` |
| Menu UI + font | `src/menu.rs`, `src/font.rs` |
| Persisted settings | `src/config.rs` |
| Savestate snapshot/restore | `src/savestate.rs`, `Nes::save_state`/`load_state` |
