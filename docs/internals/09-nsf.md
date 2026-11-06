# NSF music playback

## The format

An NSF (NES Sound Format) file is a music rip, not a cartridge: a 128-byte
header followed by the game's sound-driver code and data. The header names
three addresses - **load** (where the data goes), **init** (called once per
track with the 0-based song number in A and the region in X) and **play**
(called at a fixed rate, normally once per frame) - plus the song count,
title/artist/copyright strings, per-region play periods in microseconds,
eight bank-switch init bytes and a bitmask of expansion sound chips.
Reference: <https://www.nesdev.org/wiki/NSF>.

## The implementation

Rather than writing a separate "player", the emulator runs the NSF on the
normal machine through a synthetic board, `NsfMapper` (`src/mapper/nsf.rs`):

- **Program mapping.** When any bank-switch byte is non-zero the data is cut
  into 4KB banks selected by writes to `$5FF8-$5FFF`, padded by the load
  address's offset within its bank; otherwise it is placed linearly at the
  load address in a 32KB image. `$6000-$7FFF` is 8KB of work RAM.
- **Built-in driver.** A hand-assembled 6502 program is served from
  `$4100-$41FF` (cartridge register space no expansion chip decodes), and
  the board overrides the reset/NMI/IRQ vectors at `$FFFA-$FFFF` to point
  into it. On reset it clears `$0000-$07FF` and `$6000-$7FFF`, zeroes
  `$4000-$4013`, writes `$00` then `$0F` to `$4015` and `$40` to `$4017`,
  loads the song (`$41F0`) and region (`$41F1`) into A/X, `JSR`s to init,
  starts the play timer (write `$41F3`) and idles with interrupts enabled.
- **Play timing.** The board counts CPU cycles and raises IRQ every
  `speed_us * cpu_hz / 1e6` cycles (default 16639 us NTSC / 19997 us PAL when
  the header field is 0). The driver's IRQ handler reads `$41F2` to
  acknowledge (bit 7 = the timer fired) and `JSR`s to play. Because the real
  CPU and APU run the tune's code, its timing and sound are cycle-accurate.
- **Expansion audio.** VRC6, VRC7 (OPLL), Namco 163, Sunsoft 5B and MMC5
  (pulses, PCM, multiplier, ExRAM as plain RAM) reuse the cartridge mappers'
  own audio units (`Vrc6Audio`, `Opll`, `N163Audio`, `Sunsoft5b`,
  `Mmc5Audio`), instantiated only when the header enables them and summed
  into `Mapper::audio_sample`. FDS audio is not wired in yet.
- **Track selection.** `Nes::nsf_play(song)` calls `Mapper::nsf_select`
  (rewind banking, stop the timer, fresh expansion chips) and then resets
  the PPU, APU and CPU, so the driver re-runs init for the new track.

| What | Where |
|---|---|
| Header parsing, `NsfInfo` | `src/nsf.rs` |
| Board, driver, timer, chip routing | `NsfMapper` (`src/mapper/nsf.rs`) |
| Public API | `Nes::new_nsf`, `Nes::nsf_info`, `Nes::nsf_play` (`src/nes.rs`); `Nes::new` auto-detects NSF files |

Not emulated: FDS-chip tunes (which also need RAM at `$6000-$DFFF` and the
`$5FF6/$5FF7` banks), NSF2 metadata chunks and IRQ features, and non-timer
IRQ sources inside a tune (a DMC IRQ is ignored by the driver).
