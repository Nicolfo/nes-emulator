# NES Emulator — Review: problems, gaps, and suggestions

_Audit date: 2026-06-26_

This is a genuinely high-quality, cycle-accurate emulator (140/140 AccuracyCoin,
nestest, 43 mappers, no `TODO`/`unwrap` litter in the core, hardened ROM loader,
clean clippy/fmt/CI). The findings below are about hardening and breadth, not
rescuing a shaky base. Ordered by value.

## 1. Real bugs / robustness (fix these first) — ✅ DONE

_All items in this section were fixed; see the savestate-hardening commit. The
findings below are kept for the record, each annotated with its resolution._

### 🔴 ✅ Malformed savestate can panic the running game
The frontend catches ROM panics, but the savestate-restore path had gaps:

- `Ppu::sprite_count` (`src/ppu.rs:55`) is restored verbatim from JSON and used
  to slice a fixed `[SpriteRow; 8]` (`src/ppu.rs:798`, `:816`, `:826`). A state
  with `sprite_count > 8` → out-of-range panic on the next rendered scanline.
- Mapper restore replaces the whole struct from untrusted JSON
  (`src/mapper.rs:176`). MMC3 — and any mapper doing `len()/bank − 1` or
  `% banks` — panics on empty `prg`/`chr` (`src/mapper/mmc3.rs:54-55`, `:93`).
  The loader explicitly guards zero-PRG (`src/cartridge.rs:58`); the restore path
  did not.

**Fixed:** `Ppu::validate()` checks `sprite_count`/`sec_addr`/`scanline`/`dot`
ranges and runs in `Nes::load_state` *before* any mutation. PRG is re-injected
from the live cartridge (never read from the blob) and CHR length is validated,
so empty/short ROM in a crafted blob returns `Err` instead of panicking.

### 🔴 ✅ Savestates serialize the entire PRG/CHR ROM
The ROM `Vec<u8>`s were plain serde fields (`src/mapper/mmc3.rs:8-9`), so every
`.state` file embedded a megabytes-large copy of the ROM as JSON. This was the
root of the panic above *and* made states needlessly huge/slow.

**Fixed:** `prg` is `#[serde(skip)]` on every mapper and re-injected from the
loaded cartridge on restore; CHR-ROM is elided too on the 26 mappers that carry
a `chr_is_ram` flag (CHR-RAM stays serialized — it's genuine state). nestest's
state dropped from ROM-embedding size to ~18 KB.

> **Follow-up (not yet done):** the 10 mappers *without* a `chr_is_ram` field —
> `cnrom`, `gxrom`, `mmc2`, `mmc4`, `mmc5`, `n163`, `namco175_340`, `sunsoft4`,
> `vrc3`, `vrc6` (incl. MMC5, MMC2/4, VRC6) — still serialize their CHR-ROM into
> the blob. This is fully crash-safe and correct (CHR length is validated on
> restore), but leaves residual file bloat for those boards. Giving each a
> `chr_is_ram` flag and switching its macro call to
> `impl_mapper_savestate!(chr_is_ram = …)` would extend CHR-ROM elision
> everywhere.

### 🟠 ✅ `controller2` is never saved
`src/nes.rs:73` / `:103` snapshotted only `controller1`. Two-player state
restored wrong.

**Fixed:** `SaveState` now snapshots both ports (`controller1` + `controller2`);
savestate `VERSION` bumped to 2 so older states are rejected rather than
misread.

### 🟠 ✅ A `JAM`/`KIL` opcode hangs the emulator
`run_frame` spun until vblank (`src/nes.rs:110`); a JAM (`src/cpu.rs:1306`)
loops on `pc-1` — a pathological state could fail to reach vblank.

**Fixed:** `run_frame` now bails out after a 100k-cycle ceiling (≈3× a real
frame), so a jammed CPU can't spin forever and the frontend stays responsive.

### 🟡 ✅ Two `.unwrap()`s in the DMC path
`src/cpu.rs:400` / `:415` were safe but guard-dependent.

**Fixed:** converted to `if let Some(...)` let-chains.

## 2. Missing emulation breadth

- **Multicart/pirate mappers** (225/226/227/228/229/255, JY/90) are the largest
  real-world ROM-set gap — currently a hard error.
- **NES 2.0 RAM sizing ignored:** PRG-RAM (byte 10) and CHR-RAM (byte 11) sizes
  are never read; every CHR-RAM board hardcodes 8KB (`src/mapper/nrom.rs:14`,
  `src/mapper/mmc1.rs:32`, …). Battery-backed CHR-RAM (NVRAM) has no save path.
- **FDS and NSF** unsupported (loader is iNES-only, `src/cartridge.rs:36`) — FDS
  especially is a large library.
- **Known approximations (documented):**
  - MMC5 vertical split not emulated (`src/mapper/mmc5.rs:10`; affects
    Castlevania III).
  - Action 53 bank math self-described as untuned (`src/mapper/action53.rs:15`).
  - Bus conflicts only on ColorDreams; UNROM-180 / Crazy Climber explicitly
    affected (`src/mapper/unrom180.rs:7`); also missing on UxROM/CNROM.
  - Jaleco-72 uPD7756 sample audio (`src/mapper/jaleco_jf17.rs:16`) and some
    VRC7 envelope timing (`src/mapper/vrc7.rs:245`) missing.
  - N163 $E800 CIRAM-disable bits not emulated (`src/mapper/n163.rs:84`).
- **Submapper plumbing** is partial — passed to a few mappers, ignored by most
  (e.g. VRC4 variants resolve by mapper number only).

## 3. Missing frontend / UX features

- **No pixel-aspect-ratio correction.** The window only scales 256×240 by
  integer factors (`src/main.rs:531`); real NES output is 8:7 PAR ≈ 4:3 DAR.
  Games are subtly mis-proportioned with no option to fix.
- **No audio volume / mute control** (`src/audio.rs` has none; output is mono
  replicated to all channels).
- **No fast-forward, no rewind, no frame-step, no pause-in-place** (Esc only
  opens the menu).
- **Gamepad mapping is hardcoded** (`src/gamepad.rs:60`) — no rebinding, no
  turbo / auto-fire.
- **No recent-ROMs list, no savestate thumbnails** (framebuffer is excluded from
  states), **no region override** (region is auto-only).
- Only 4 savestate slots; no quick-save/quick-load hotkey distinct from the
  picker.

## 4. Test / CI gaps

- **36 Holy Mapperel + 4 boot_smoke tests never run anywhere** — they silently
  skip (no ROMs) locally *and* in CI, since CI only fetches AccuracyCoin
  (`.github/workflows/ci.yml:111`). 40+ "passing" tests are no-ops. Highest-value
  fix: have CI fetch the Holy Mapperel ROMs the same way.
- **`src/config.rs` has zero tests** despite recent persistent-config work;
  `gamepad.rs`, `audio.rs`, `palette.rs` untested.
- **No PPU golden-frame test** (only register-level checks) and **no APU
  mixer-level test** ("non-silent" only).
- `tests/accuracycoin_rom.rs` uses brittle RAM-flag / frame-budget heuristics —
  the one real flakiness risk.
- clippy / fmt are Linux-only, so Windows `#[cfg]` paths go unlinted.

## Suggested priority order

1. Harden savestate restore + stop embedding the ROM (`#[serde(skip)]`) — fixes a
   crash class *and* file bloat at once.
2. Wire Holy Mapperel ROMs into CI so 36 dead mapper tests start giving signal.
3. Add aspect-ratio correction + audio volume + fast-forward — the most-felt UX
   gaps.
4. NES 2.0 RAM sizing + a batch of multicart mappers for library coverage.
5. Save `controller2`; add a JAM / runaway-frame ceiling.
