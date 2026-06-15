# Mapper TODO

Tracks mappers left to implement. Dispatch table: `src/cartridge.rs`
(`match mapper_id`). Each mapper lives in `src/mapper/`.

## Implemented

| # | Board | File |
|---|-------|------|
| 0 | NROM | `nrom.rs` |
| 1 | MMC1 (SxROM) | `mmc1.rs` |
| 2 | UxROM | `uxrom.rs` |
| 3 | CNROM | `cnrom.rs` |
| 4 | MMC3 (TxROM) | `mmc3.rs` |
| 5 | MMC5 (ExROM) | `mmc5.rs` |
| 7 | AxROM | `axrom.rs` |
| 9 | MMC2 (PxROM) | `mmc2.rs` |
| 10 | MMC4 (FxROM) | `mmc4.rs` |
| 11 | Color Dreams | `colordreams.rs` |
| 19 | Namco 163 | `n163.rs` |
| 24 | VRC6a | `vrc6.rs` |
| 26 | VRC6b | `vrc6.rs` |
| 28 | Action 53 | `action53.rs` |
| 34 | BNROM / NINA-001 | `bnrom.rs` |
| 66 | GxROM | `gxrom.rs` |
| 69 | Sunsoft FME-7 | `fme7.rs` |
| 71 | Codemasters | `codemasters.rs` |
| 78 | Holy Diver / Cosmo Carrier | `holydiver.rs` |
| 118 | TxSROM | `txsrom.rs` |
| 180 | UNROM 180 (Crazy Climber) | `unrom180.rs` |
| 206 | Namco 108 | `namco108.rs` |

## TODO

Mappers worth adding. None have board-test ROMs in `testroms/` yet, so
verify against real game ROMs.

| # | Board | Notes |
|---|-------|-------|
| 21/22/23/25 | VRC4 / VRC2 | Konami; many JP titles. Share core w/ VRC6 IRQ. |
| 16/159 | Bandai FCG / LZ93D50 | Dragon Ball, EEPROM saves. |
| 64 | RAMBO-1 (Tengen) | MMC3-like + extra IRQ mode. |
| 65 | Irem H3001 | IRQ + banking. |
| 68 | Sunsoft-4 | Dual NT CHR-ROM mirroring. |
| 73 | VRC3 | Salamander. |
| 75 | VRC1 | |
| 85 | VRC7 | FM expansion audio (Lagrange Point). Big job. |
| 210 | Namco 175/340 | N163 variants w/o audio. |

## Notes

- Add to dispatch in `src/cartridge.rs`, register module in `src/mapper.rs`.
- Implement `impl_mapper_savestate!()` for savestate support.
- After adding, drop board-test ROM(s) into `tests/holy_mapperel.rs` `hm_test!`.
