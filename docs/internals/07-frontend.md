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
| `snapshot` / `restore_snapshot` | In-memory snapshots for rewind (mapper JSON appended raw, not re-encoded) |
| `new_with_region` | Build with a forced TV system (NTSC / PAL / Dendy) |
| `new_fds` / `fds_insert` / `fds_side_count` | Famicom Disk System boot and disk swapping |
| `new_nsf` / `nsf_info` / `nsf_play` | NSF music playback ([chapter 8](09-nsf.md)) |
| `set_buttons` / `set_input_device` / `set_zapper` | Controller ports: up to 4 pads, Four Score, Zapper |
| `set_cheats` | Game Genie / raw cheat codes |

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
write strobes both at once; reads come back on the matching port. The host feeds
both ports from two input sources merged per player: the keyboard
(`cfg.keys` / `cfg.keys_p2`) and an optional physical gamepad (see below).

The host doesn't call the shift register directly during gameplay - it just sets
the merged button state (`Nes::set_buttons`, once per emulated frame), and the *bus* clocks the
strobe/shift at the right cycle parity (`clock_put_cycle`, called from
`tick_cycle_post`; see [chapter 4](05-bus-timing-dma.md)). The
reload-on-put-cycle detail is what makes the shifting cycle-accurate.

### Four Score and Zapper

`InputDevice` (in `controller.rs`) picks what is plugged into the ports, chosen
in Settings > OPTIONS > INPUT:

- **2 PADS** - the standard joypads above.
- **FOUR SCORE** - each port carries two pads followed by an 8-bit signature,
  so each `Controller` switches to a 24-bit shift register: P1, P3 and `$10`
  on `$4016`; P2, P4 and `$20` on `$4017` (read MSB-first, as games do). The
  wiring is host configuration, not machine state: savestates restore the
  button latches but keep the live wiring.
- **ZAPPER** - [`src/zapper.rs`](../../src/zapper.rs) answers `$4017` instead
  of pad 2: D4 is the trigger, D3 the light sensor (0 = light). The sensor
  looks at the pixels around the aim point that the PPU drew in the last ~20
  scanlines of the *current* frame, mimicking the photodiode's short
  persistence; that is exactly what Duck Hunt's "flash a white box over the
  target" detection relies on. The mouse aims (window coordinates mapped back
  through `pixels::window_pos_to_pixel` plus the overscan crop), left click
  pulls the trigger, right click fires off screen.

### Turbo buttons

Each player has TURBO A / TURBO B bindings (and gamepad North/West). While
held they toggle the button every 2 emulated frames (15 presses a second),
driven by `App::frame_count` so fast-forward and rewind stay in step.

### Physical gamepads

[`src/gamepad.rs`](../../src/gamepad.rs) adds optional gamepad input via
[gilrs](https://docs.rs/gilrs). Pads map to players 1-4 in plug order
(hot-plug tracked; players 3-4 reach the game through a Four Score); South/East are A/B,
North/West are turbo A/B,
Start/Select map across, and directions come from the D-pad or the left analog
stick. Each player's keyboard and gamepad inputs are tracked as **separate
bitmasks** and OR-ed together by `App::frame_inputs` before being written to the
controller, so neither source clears the other's held buttons. `about_to_wait`
polls the pads once per frame; key events update the keyboard mask. If gilrs
can't initialise (no input backend), the frontend silently stays keyboard-only.

## The host application (`src/main.rs`)

The frontend is a [winit](https://docs.rs/winit) 0.30 + [pixels](https://docs.rs/pixels)
application - a small state machine with four views:

```rust
enum View { Home { .. }, Settings { .. }, SlotPicker { .. }, Cheats { .. }, Running }
```

- **Home / Settings** render the NES-style menu UI (drawn into the same 256×240
  framebuffer by `src/menu.rs` using the embedded bitmap font in `src/font.rs`).
  Settings has two pages. CONTROLS rebinds keys per player (an EDIT PLAYER
  toggle switches between player 1 and player 2), including the turbo
  buttons. OPTIONS holds window scale, NTSC overscan cropping, volume,
  fast-forward speed, input device, region override and rewind on/off - all
  persisted via `src/config.rs` (older config files load with defaults for
  the new fields).
- **Cheats** lists the loaded game's codes (Enter toggles, Delete removes,
  ADD CODE types a new one). See Cheats below.
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

### Fast-forward and rewind

Holding **Tab** runs `cfg.ff_speed` (2-8) frames per displayed frame. The
audio keeps every Nth sample, so it plays sped-up and higher-pitched in real
time instead of piling up latency.

With rewind enabled, `emulate_frame` takes a `Nes::snapshot` every 2 frames
into a [`Rewind`](../../src/rewind.rs) ring of 600 (20 seconds). Snapshots are
JSON text, so consecutive ones are near-identical but shift around whenever a
value changes digit count. The ring splits each one with content-defined
chunking (a gear rolling hash picks cut points from the bytes themselves, so
cuts re-synchronise right after an edit) and stores each distinct ~1 KB chunk
once, refcounted. 20 seconds of SMB3 costs about 4 MB instead of ~260 MB.
Holding **Backspace** pops one snapshot per displayed frame, restores it and
runs one frame to redraw - rewinding at 2x speed, muted.

### Status overlay

`App::flash` shows a short message (state saved/loaded, reset, mute, disk
side) for two seconds in the top-left corner via `menu::draw_osd`; fast-forward
and rewind show their own indicator while held.

### Cheats

[`src/cheats.rs`](../../src/cheats.rs) decodes Game Genie codes (6 letters,
or 8 with a compare byte) and raw `AAAA:VV` / `AAAA?CC:VV` codes. ROM-range
codes become `RomPatch`es checked on cartridge reads in `Bus::read` (one
`is_empty` test on the hot path when none are active); RAM-range codes are
poked into RAM at the start of every frame. The list lives next to the ROM as
`<rom>.cht`, one code per line, `-` prefix for disabled ones.

### Famicom Disk System and NSF

`App::open_rom` sniffs the file: FDS images (`FDS\x1A` header or a raw
`*NINTENDO-HVC*` dump, or a `.fds` extension) boot through `Nes::new_fds`
with the BIOS, which users must supply as `disksys.rom` next to the image, in
the config directory or next to the executable. **F8** flips to the next disk
side (the drive stays empty ~1 s so the BIOS sees the change). The disk
image itself is the "battery RAM": writes the game makes are saved to
`<image>.sav` in plain `.fds` layout.

NSF files (`NESM\x1A`) load through `Nes::new` and show a player screen
(`menu::render_nsf`) instead of the framebuffer: title, artist, track
number and play time. Left/Right switch tracks, F3 restarts the current one.

### Region override

Settings > OPTIONS > REGION forces NTSC, PAL or **Dendy** (AUTO follows the
header; NES 2.0 timing byte 3 already selects Dendy). Changing it restarts a
running cartridge game from power-on in the new region (battery save flushed
first); FDS and NSF ignore it. The
Dendy is a PAL-clocked Famiclone: 312-line 50 Hz frames like PAL, but 3 PPU
dots per CPU cycle like NTSC, 51 idle post-render lines with vblank starting
at line 291, and NTSC APU tables at 1.773 MHz.

### Audio output and dynamic rate control

[`src/audio.rs`](../../src/audio.rs) opens a [cpal](https://docs.rs/cpal) output
stream and feeds it from a shared sample queue; underruns decay to silence to
avoid clicks. The callback multiplies by a gain shared through an atomic,
so the VOLUME setting and **F6** mute apply instantly. The interesting bit is in `about_to_wait`: the emulator generates
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
| Physical gamepad input | `src/gamepad.rs` |
| Zapper light sensing | `src/zapper.rs` |
| Cheat code decoding | `src/cheats.rs` |
| Rewind history | `src/rewind.rs`, `Nes::snapshot` |
| Window / input / view state machine | `src/main.rs` |
| Frame pacing + audio rate control | `about_to_wait` (main.rs) |
| Audio device output | `src/audio.rs` |
| Menu UI + font | `src/menu.rs`, `src/font.rs` |
| Persisted settings | `src/config.rs` |
| Savestate snapshot/restore | `src/savestate.rs`, `Nes::save_state`/`load_state` |
