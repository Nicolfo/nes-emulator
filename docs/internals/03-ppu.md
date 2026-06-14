# 2 — The PPU (picture processing unit)

The PPU is the largest and trickiest part of the machine. This chapter builds up
the model in layers: first what it draws and when, then how the scroll registers
work, then the background and sprite pipelines, then the register interface the
CPU uses to talk to it.

## The hardware

### The raster: scanlines and dots

The PPU produces a **256×240** image the way a CRT does: it sweeps an imaginary
beam left to right, top to bottom. Time is measured in **dots** (one dot ≈ one
pixel) and **scanlines** (one row).

A full NTSC frame is laid out as:

| Scanline | Name | What happens |
|----------|------|--------------|
| `-1` (261) | **pre-render** | Dummy line; clears flags, refills the pipeline for line 0 |
| `0`–`239` | **visible** | The 240 rows that actually appear on screen |
| `240` | post-render | Idle |
| `241`–`260` | **vblank** | Beam "off-screen"; the CPU can safely touch PPU memory. NMI fires at 241 |

Each scanline is **341 dots** (0–340). So a frame is 262 scanlines × 341 dots ≈
89,342 dots. PAL is taller (262 → 312 scanlines) with a longer vblank.

Why this matters: the CPU can only freely write video memory during vblank
(~2273 CPU cycles). Everything a game does to the picture is paced against this
schedule, and advanced games change PPU state *mid-scanline* to do things like
split the screen.

### What's in PPU memory

The PPU's 16 KB address space (`$0000`–`$3FFF`) holds:

- **Pattern tables** (`$0000`–`$1FFF`) — the actual 8×8 tile bitmaps ("CHR").
  Each tile is 16 bytes: 8 bytes for bit-plane 0, 8 for bit-plane 1; combined
  they give each pixel a 2-bit value (0–3). This is on the cartridge.
- **Nametables** (`$2000`–`$2FFF`) — the background layout: a grid of tile
  indices plus **attribute** bytes that assign a palette to each 16×16 region.
  These live in the console's 2 KB video RAM (CIRAM), but how the four logical
  nametables map onto that 2 KB is decided by the cartridge's **mirroring** (see
  [chapter 5](06-cartridge-mappers.md)).
- **Palette** (`$3F00`–`$3F1F`) — 32 entries selecting from the NES's master
  list of ~64 colors. One backdrop color, four background sub-palettes, four
  sprite sub-palettes.

### Two things drawn per pixel

For each visible dot the PPU computes a **background** pixel and a **sprite**
pixel and decides which wins:

- **Background** comes from the nametable + attribute + pattern, positioned by
  the scroll registers.
- **Sprites** (up to 64, defined in **OAM** — object attribute memory, 256 bytes)
  are independently positioned 8×8 or 8×16 tiles. Only **8 sprites per scanline**
  can be drawn; the 9th sets an overflow flag.

Priority, transparency (color 0 = transparent), and the special **sprite-zero
hit** (set when sprite 0 and the background both have an opaque pixel at the same
spot — games use it as a mid-frame timing beacon) all resolve here.

### The loopy registers

Scrolling is implemented with two 15-bit internal registers, conventionally
named after the person who reverse-engineered them (`v`, `t`), plus `fine_x` and
a write toggle `w`:

- **`v`** — the "current" VRAM address used during rendering. Its bits encode
  coarse-X, coarse-Y, the nametable select, and fine-Y scroll, all packed
  together so the rendering hardware can increment them cheaply.
- **`t`** — a "temporary" address; the staging copy that scroll writes go into.
- **`fine_x`** — the sub-tile horizontal scroll (0–7).
- **`w`** — toggles between the first and second write of the two-write registers
  (`$2005` scroll and `$2006` address).

This packed layout is why a single hardware increment can step the scroll through
tiles and wrap across nametables.

## The implementation

The PPU is **dot-stepped**: [`Ppu::tick`](../../src/ppu.rs) advances the machine
by exactly one dot, and the bus calls it three times per CPU cycle. The whole
chip is one `tick` driving a small set of per-dot routines.

### State

The `Ppu` struct holds the loopy registers (`v, t, fine_x, w`), the memory-mapped
registers (`ctrl, mask, status`), the background pipeline shifters/latches, the
sprite pipeline state, OAM, secondary OAM, palette, CIRAM (`vram`), the
`scanline`/`dot` counters, and the RGBA `framebuffer` that the frontend reads.

### The per-dot tick

[`Ppu::tick`](../../src/ppu.rs) is the heart. Each call, in order, it:

1. Applies any pending `$2001` (mask) write that is now due (writes take effect a
   few dots late — see Quirks).
2. Advances the `$2007` data-bus state machine (see "The register interface").
3. Applies any pending OAM corruption.
4. If rendering is on and we're on a visible/pre-render line, runs one dot of the
   **sprite pipeline** ([`sprite_pipeline_dot`](../../src/ppu.rs)) and the
   **background pipeline** (shift + `bg_fetch`).
5. Performs the scroll increments at their specific dots (256: increment Y; 257:
   copy horizontal bits from `t`; 280–304 on pre-render: copy vertical bits).
6. On visible dots 1–256, draws the pixel ([`render_pixel`](../../src/ppu.rs))
   and clocks the sprite X-counters.
7. Sets the vblank flag / `frame_complete` at scanline 241 dot 1.
8. Increments `dot`/`scanline`, wrapping the frame and toggling `odd_frame`,
   handling the NTSC odd-frame skipped dot.

### Background pipeline

The background is produced by a small assembly line, exactly like the hardware's
shift registers. Over each 8-dot span, [`bg_fetch`](../../src/ppu.rs) performs
four memory fetches in sequence:

- dot %8 == 0: the **nametable** byte (which tile) — and reload the shifters
- dot %8 == 2: the **attribute** byte (which palette)
- dot %8 == 4: pattern **low** plane
- dot %8 == 6: pattern **high** plane
- dot %8 == 7: increment coarse-X in `v`

The fetched bytes go into latches (`nt_latch`, `at_latch`, `pat_lo_latch`,
`pat_hi_latch`), and [`load_shifters`](../../src/ppu.rs) feeds them into the
16-bit shift registers (`bg_pat_lo/hi`, `bg_attr_lo/hi`). Every drawing dot,
[`shift`](../../src/ppu.rs) shifts these one position, and `render_pixel` reads
the bit selected by `fine_x` (`bit = 15 - fine_x`). The loopy address math —
[`increment_coarse_x`](../../src/ppu.rs), [`increment_y`](../../src/ppu.rs),
[`copy_horizontal`](../../src/ppu.rs), [`copy_vertical`](../../src/ppu.rs) — is a
direct transcription of the documented register operations.

> **Quirk — shifter serial inputs.** `shift` doesn't just shift in zeros; it
> shifts a 0 into the low plane and a 1 into the high plane, mirroring real
> silicon. This is invisible normally (reloads overwrite those bits) but becomes
> visible when rendering is blanked right around the reload dot — which some test
> ROMs check.

### Sprite pipeline

The sprite logic is modeled **dot-accurately** through *secondary OAM*, the
hardware's 32-byte scratch buffer for the (up to) 8 sprites on the next line.
[`sprite_pipeline_dot`](../../src/ppu.rs) reproduces the three phases of a
scanline:

1. **Dots 1–64 — clear.** Secondary OAM is initialized to `$FF`; `$2004` reads
   return `$FF`.
2. **Dots 65–256 — evaluation.** Walk primary OAM looking for sprites in range
   for the next line, copying their 4 bytes into secondary OAM, stopping at 8.
   This is implemented at the same 2-dot read/write cadence the hardware uses,
   starting from the live `OAMADDR` (so a misaligned `OAMADDR` misaligns every
   sprite, exactly as on hardware), including the **buggy overflow scan** (the
   "diagonal" where both indices increment) that makes the sprite-overflow flag
   famously unreliable.
3. **Dots 257–320 — fetch.** For each of the 8 slots, fetch the pattern bytes
   ([`sprite_pat_addr`](../../src/ppu.rs), handling 8×16 sprites and vertical
   flip) and load them into the eight `SpriteRow` shifters.

During drawing, each sprite has an **X down-counter** that ticks every dot
([`clock_sprite_counters`](../../src/ppu.rs)); when it reaches zero the sprite's
shifter starts outputting. [`sprite_pixel`](../../src/ppu.rs) returns the first
opaque sprite pixel and whether it is sprite zero / behind the background.
`render_pixel` combines background and sprite per the priority rules and sets the
**sprite-zero hit** flag when sprite 0 and background pixels are both opaque
(except at x=255).

> **Quirk — counting/halted sprite modes & OAM corruption.** The `SpriteRow`
> `counting` flag and `arm_sprite_counters` reproduce the exact dot (339) on
> which counters are armed, and `pending_corruption` models the OAM corruption
> that real hardware exhibits when rendering is disabled mid-line. These are the
> kinds of details AccuracyCoin checks; they are not needed to run ordinary
> games but are faithfully present.

### The register interface ($2000–$2007)

The CPU's only window into the PPU is eight registers, mirrored every 8 bytes
across `$2000`–`$3FFF`. The bus routes reads/writes here to
[`read_register`](../../src/ppu.rs) / [`write_register`](../../src/ppu.rs):

| Addr | Name | Direction | Role |
|------|------|-----------|------|
| `$2000` | PPUCTRL | write | NMI enable, sprite size, base nametable, `$2007` increment |
| `$2001` | PPUMASK | write | Rendering enable, greyscale, clipping |
| `$2002` | PPUSTATUS | read | vblank / sprite-0 / overflow flags; reading clears vblank + the `w` toggle |
| `$2003` | OAMADDR | write | OAM pointer |
| `$2004` | OAMDATA | read/write | OAM access |
| `$2005` | PPUSCROLL | write ×2 | Scroll (writes into `t`/`fine_x`) |
| `$2006` | PPUADDR | write ×2 | VRAM address (writes into `t`, then `v`) |
| `$2007` | PPUDATA | read/write | VRAM data at `v`, auto-incrementing |

A few implementation points worth calling out:

- **`$2005`/`$2006` share the `w` toggle and write into `t`.** The exact bit
  manipulations in `write_register` cases 5 and 6 are the canonical loopy
  formulas. `$2006`'s second write copies `t` into `v`.
- **Buffered `$2007` reads.** Reading VRAM (outside the palette) returns the
  *previous* buffered byte and then refills the buffer — a one-read delay that
  matches the hardware's internal read buffer. Palette reads are immediate but
  still update the buffer with the nametable byte "underneath."
- **Open bus / I/O decay.** The PPU data bus is a capacitive latch: bits that
  aren't refreshed decay toward 0 over ~25 frames. `io_bus`, `io_bus_ts`,
  `io_bus_read`, and `io_bus_refresh` model this per-bit analog decay, so reads
  of write-only registers return the plausible "open bus" value.

> **Quirk — the $2007 data-bus state machine.** When the CPU reads `$2007`
> *during* active rendering, the buffer refill doesn't happen immediately; it
> goes through a small state machine that fires a few dots later from whatever
> the rendering pipeline is driving on the bus, and can collide with a pipeline
> fetch (`bus_conflict`, `capture_delay`, `last_fetch_val`). The `v` increment
> also glitches into a simultaneous coarse-X + Y increment
> ([`increment_v_after_2007`](../../src/ppu.rs)). This is exotic but required for
> full accuracy.

> **Quirk — vblank/NMI suppression race.** Reading `$2002` one PPU clock before
> the vblank flag is set returns it clear *and* suppresses it for the whole
> frame (`suppress_vbl`), which kills the NMI. The pre-render-line read also sees
> the sprite flags as already cleared. Both are handled in `read_register` case
> 2 and at scanline 241 dot 1 in `tick`.

> **Quirk — odd-frame dot skip.** On NTSC, when rendering is enabled, odd frames
> are one dot shorter — the pre-render line skips its last dot. This keeps the
> color subcarrier phase consistent on a real TV. The `skip` logic lives at the
> end of `tick`; PAL has no such skip.

### Producing the picture

[`render_pixel`](../../src/ppu.rs) writes RGBA into `framebuffer` by looking up
the resolved palette index in [`NES_PALETTE`](../../src/palette.rs) (the
hardcoded 64-color → RGB table). Greyscale mode (`mask & 1`) masks the low bits
of the palette index. The frontend later copies this buffer to the screen, with
optional overscan cropping (see [chapter 6](07-frontend.md)).

### Where to look

| You want to understand… | Look at |
|---|---|
| The frame schedule | `tick`, `last_line`, the scanline/dot wrap |
| Scrolling math | `increment_coarse_x/y`, `copy_horizontal/vertical`, `write_register` 5/6 |
| Background rendering | `bg_fetch`, `load_shifters`, `shift`, `render_pixel` |
| Sprite evaluation | `sprite_pipeline_dot`, `sprite_pat_addr`, `load_sprite_slot` |
| Sprite drawing | `clock_sprite_counters`, `sprite_pixel` |
| CPU↔PPU registers | `read_register`, `write_register` |
| Open-bus decay | `io_bus_read`, `io_bus_refresh` |
