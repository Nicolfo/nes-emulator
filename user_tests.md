# Manual mapper test plan

Games chosen to stress each mapper's riskiest feature, not just to boot.
Run with `cargo run --release <rom>`.

**Priority pass (one game per new mapper, riskiest feature first):**

1. *Mike Tyson's Punch-Out!!* (9) — CHR latch
2. *Castlevania III* (5) — scanline IRQ
3. *Akumajou Densetsu* (24) — IRQ + audio
4. *Gimmick!* (69) — IRQ + 5B audio
5. *Rolling Thunder (J)* (19) — CHR-ROM nametables

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

## Test ROMs (non-game)

- **[Holy Mapperel](https://github.com/pinobatch/holy-mapperel)** (tepples) —
  automatic pass/fail screens for mappers 0–4, 7, 9, 11, 66, 69: PRG/CHR/WRAM
  and mirroring checks. Best candidate for an automated framebuffer-hash test
  in the style of `tests/boot_smoke.rs`.
- Audio comparison: record against Mesen/NSFPlay for VRC6, N163, 5B and MMC5
  channel balance — the per-chip mix constants are tunable starting points.
