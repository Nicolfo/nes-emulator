# Manual mapper test plan

Games chosen to stress each mapper's riskiest feature, not just to boot.
Run with `cargo run --release <rom>`.

**Priority pass (one game per new mapper, riskiest feature first):**

1. *Mike Tyson's Punch-Out!!* (9) — CHR latch
2. *Castlevania III* (5) — scanline IRQ
3. *Akumajou Densetsu* (24) — IRQ + audio
4. *Gimmick!* (69) — IRQ + 5B audio
5. *Rolling Thunder (J)* (19) — CHR-ROM nametables
6. *Fire Emblem (J)* (10) — MMC4 CHR latch + 16KB PRG banking + battery RAM
7. *Madara (J)* (26) — VRC6b swapped-pinout audio + IRQ

## Mapper 0 — NROM

| Game | What to check |
|---|---|
| Super Mario Bros. | Baseline scrolling, sprite-0 hit (status bar) |
| Donkey Kong, Balloon Fight | Trivial sanity |

## Mapper 1 — MMC1

| Game | What to check |
|---|---|
| The Legend of Zelda | CHR-RAM, battery RAM, mirroring switches on screen transitions |
| Metroid | Mid-game bank switching |
| Final Fantasy / Dragon Warrior IV | Battery saves (in-session only — no `.sav` persistence yet) |

## Mapper 2 — UxROM

| Game | What to check |
|---|---|
| Mega Man | Bank switching + CHR-RAM |
| Castlevania, Contra | Scroll-heavy gameplay |

## Mapper 3 — CNROM

| Game | What to check |
|---|---|
| Solomon's Key, Arkanoid | CHR bank selects |

## Mapper 4 — MMC3

| Game | What to check |
|---|---|
| Super Mario Bros. 3 | A12 IRQ status bar, heavy banking |
| Battletoads | Timing-sensitive (already in `tests/boot_smoke.rs`) |
| Kirby's Adventure | IRQ split + CHR-RAM mixing |
| Mega Man 3 | IRQ edge cases (boss intro splits) |

## Mapper 5 — MMC5 (new)

| Game | What to check |
|---|---|
| Castlevania III (US) | **The test.** Scanline-IRQ status bar, PRG/CHR mode switching, 8x16 sprite CHR sets, pulse audio. If the status bar shakes or splits drift, suspect the NT-fetch streak logic in `src/mapper/mmc5.rs` (`nt_target`). |
| Uncharted Waters | ExGrafix extended attributes (per-tile CHR banking) + banked PRG RAM ($5113) |
| Just Breed | PCM audio + ExGrafix |
| Laser Invasion | Split status + audio |

Known-broken by design: anything using vertical split ($5200–$5202).
Known quirk: the first two tiles of the top scanline can glitch in ExGrafix
games (prefetched on the pre-render line before in-frame is established);
hidden by TV overscan.

## Mapper 7 — AxROM

| Game | What to check |
|---|---|
| Solstice | Single-screen mirroring switches |
| Wizards & Warriors, Marble Madness | General banking |

## Mapper 9 — MMC2 (new)

| Game | What to check |
|---|---|
| Mike Tyson's Punch-Out!! | Only game that matters. FD/FE latches flip mid-screen (big opponent sprites split across pattern tables). Garbled ring/opponent tiles during a fight = latch bug. |

## Mapper 10 — MMC4 (new)

| Game | What to check |
|---|---|
| Fire Emblem (J) | The test. FD/FE CHR latches (same as MMC2) over 16KB PRG banking; battery PRG RAM at $6000. Garbled portraits/map tiles = latch bug; lost saves = PRG RAM bug. |
| Fire Emblem Gaiden (J) | More CHR-latch stress, longer play sessions for battery RAM |
| Famicom Wars (J) | 16KB PRG bank switching + latch tiles |

## Mapper 11 — Color Dreams (new)

| Game | What to check |
|---|---|
| Crystal Mines, Bible Adventures | Bank select + bus-conflict AND |

## Mapper 19 — Namco 163 (new)

| Game | What to check |
|---|---|
| Rolling Thunder (J) | CHR-ROM nametables (`nt_target` Cart path) — title screen breaks if NT mapping is wrong |
| Megami Tensei II (J) | Wavetable audio + IRQ |
| Final Lap (J) | NT-from-CHR + audio |
| Mappy Kids (J) | Multi-channel wavetable stress |

## Mapper 24 — VRC6 (new)

| Game | What to check |
|---|---|
| Akumajou Densetsu (J) | Definitive: scanline-IRQ splits + all three audio channels. Compare music against an NSF recording — the sawtooth is most audible in the stage 1 theme. |

## Mapper 26 — VRC6b (new)

| Game | What to check |
|---|---|
| Madara (J) | Same VRC6 silicon as mapper 24 on the swapped A0/A1 pinout. If audio/IRQ work on 24 but registers land wrong here, suspect the `reg()` line-swap in `src/mapper/vrc6.rs`. |
| Esper Dream 2 (J) | Audio + scanline-IRQ splits via the VRC6b decode |

## Mapper 34 — BNROM / NINA-001 (new)

| Game | What to check |
|---|---|
| Deadly Towers | BNROM path: 32KB PRG select by any $8000-$FFFF write, 8KB CHR RAM. Wrong bank = wrong level/graphics. |
| Impossible Mission II | NINA-001 path: $7FFD PRG + $7FFE/$7FFF 4KB CHR banks, PRG RAM at $6000. CHR comes from ROM here, so corrupt tiles = CHR bank bug. |
| Mashou (J) | BNROM PRG banking sanity |

## Mapper 66 — GxROM

| Game | What to check |
|---|---|
| Super Mario Bros. + Duck Hunt | Combined PRG/CHR register |
| Dragon Power | General banking |

## Mapper 69 — FME-7 (new)

| Game | What to check |
|---|---|
| Gimmick! (J) | 5B audio (tone channels carry the melody) + IRQ splits |
| Batman: Return of the Joker | IRQ-heavy parallax, plain FME-7 (no 5B audio) — clean IRQ-only check |
| Hebereke (J) | WRAM control states (ROM/RAM/open-bus at $6000) |

## Mapper 71 — Camerica / Codemasters (new)

| Game | What to check |
|---|---|
| Micro Machines | UxROM-like 16KB PRG select at $C000-$FFFF, fixed last bank, CHR RAM. Wrong bank = title/menu corruption. |
| Fire Hawk | Single-screen mirroring driven from $9000-$9FFF bit 4 — the one game that exercises it. Split/torn status overlay = mirroring-write bug. |
| Bee 52, Big Nose the Caveman | General PRG banking + CHR-RAM rendering |

## Mapper 206 — Namco 108 / DxROM (new)

| Game | What to check |
|---|---|
| Gauntlet | MMC3-style $8000/$8001 banking in fixed mode, hardwired mirroring, no IRQ. Wrong tiles/PRG = bank-register masking bug (CHR 6-bit, PRG 4-bit). |
| Pac-Mania, Dragon Buster (J) | 2KB+1KB CHR banking layout, PRG R6/R7 switching |

## Test ROMs (non-game)

- **[Holy Mapperel](https://github.com/pinobatch/holy-mapperel)** (tepples) —
  automated in `tests/holy_mapperel.rs` (ROMs in `testroms/`, mappers 0–4, 7,
  9, 11, 66, 69). Mappers 10 (MMC4) and 34 (BNROM/NINA-001) are now supported
  and can be wired into the harness once their exact ROM filenames are
  confirmed. ROMs for still-unsupported mappers (28, 78.3, 118, 180) sit in
  `testroms/` waiting for mapper support.
- Audio comparison: record against Mesen/NSFPlay for VRC6, N163, 5B and MMC5
  channel balance — the per-chip mix constants are tunable starting points.
