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

**NSF2** (header version 2) adds a feature-flags byte at `$7C` - IRQ timer
support (bit 4), non-returning init (bit 5), suppressed play (bit 6),
mandatory metadata (bit 7) - and a 24-bit program length at `$7D-$7F`; when
it is non-zero, NSFe-style metadata chunks follow the program.
**NSFe** files (`NSFE` magic) carry everything in chunks: `INFO`, `DATA`,
`BANK`, `RATE`, `NSF2` and the same metadata chunks. References:
<https://www.nesdev.org/wiki/NSF2>, <https://www.nesdev.org/wiki/NSFe>.

### Metadata

`nsf::parse` reads `auth` (title, artist, copyright, ripper - overriding the
header strings), `tlbl` (track labels), `taut` (track authors), `time` and
`fade` (track length / fade-out in ms; negative = unknown), `plst` (play
order), `RATE` (play periods) and `regn` (region, including Dendy) into
`NsfHeader::meta`. Unknown lowercase chunks are skipped; an unknown uppercase
(mandatory) chunk drops the NSF2 metadata but still plays the tune (an NSFe
file with one is rejected). Text is UTF-8 - NSF v1 header strings fall back
to Windows-1252 - and `nsf::to_display_ascii` transliterates it for the
ASCII UI font (accents stripped, fullwidth forms narrowed, typographic
punctuation simplified, other scripts shown as `?`).

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
  into it. On reset it clears `$0000-$07FF` (the board zeroes
  `$6000-$7FFF` itself, so tracks start without a RAM-clear delay), zeroes
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
  `Mmc5Audio`, `FdsAudio`), instantiated only when the header enables them
  and summed into `Mapper::audio_sample`. FDS tunes run from RAM: `$6000-$FFFF`
  is loaded from the program image (banks copied in by `$5FF6-$5FFF` writes).
- **NSF2 driver.** Tunes flagged for the IRQ timer or a non-returning init
  own the IRQ, so the board serves a different driver tail from `$4156`:
  init is called with Y = `$80`, the play timer starts, and for a
  non-returning init it is called again with Y = `$81` (`$41F4` gives the
  song without reloading FDS RAM) and may loop forever. Play then runs from
  an **NMI** the board raises at the play rate (`Mapper::nmi`, ORed into the
  CPU's NMI line): the handler at `$4172` acknowledges `$41F2`, calls play
  and writes `$41F6` when done; a period that expires during play is
  deferred, never nested. The driver never touches the I flag. `$FFFE/$FFFF`
  becomes RAM seeded from the image, and `$401B/$401C` (reload) / `$401D`
  (bit 0 enable; read = bit 7 IRQ flag, bit 0 enabled, acknowledges) form the
  NSF2 IRQ timer: it counts down every CPU cycle, fires every reload + 1
  cycles and reloads while disabled (`Mapper::apu_io_read`/`apu_io_write`
  route `$4018-$401F`). Suppressed play stops the play timer altogether.
- **Track selection.** `Nes::nsf_play(song)` calls `Mapper::nsf_select`
  (rewind banking, stop the timer, fresh expansion chips) and then resets
  the PPU, APU and CPU, so the driver re-runs init for the new track.

| What | Where |
|---|---|
| Header parsing, `NsfInfo` | `src/nsf.rs` |
| Board, driver, timer, chip routing | `NsfMapper` (`src/mapper/nsf.rs`) |
| Public API | `Nes::new_nsf`, `Nes::nsf_info`, `Nes::nsf_play` (`src/nes.rs`); `Nes::new` auto-detects NSF files |

The frontend shows the track label and length, steps through the `plst`
order with Left/Right, and when a track's length is known fades it out over
its `fade` time (2 s by default) and moves on to the next track.

Not emulated: custom `VRC7` patch sets (the default patches play),
`mixe` mixing levels, and, for classic (non-NSF2) tunes, IRQ sources other
than the play timer (a DMC IRQ is ignored by the driver).
