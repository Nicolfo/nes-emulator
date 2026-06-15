# Supported mappers

The emulator implements the following iNES/NES 2.0 mappers. "Expansion audio"
marks boards whose extra sound channels are mixed into the APU output.

| # | Board / chip | Summary | Notable games |
|---|---|---|---|
| 0 | NROM | No banking; 16/32KB PRG, 8KB CHR | Super Mario Bros., Donkey Kong, Balloon Fight |
| 1 | MMC1 | Serial-loaded PRG/CHR banking, switchable mirroring, battery PRG RAM | The Legend of Zelda, Metroid, Final Fantasy |
| 2 | UxROM | 16KB switchable PRG + fixed last bank, CHR RAM | Mega Man, Castlevania, Contra |
| 3 | CNROM | Fixed PRG, 8KB CHR bank select | Solomon's Key, Arkanoid |
| 4 | MMC3 | 8KB PRG / 1KB CHR banking, A12 scanline IRQ, battery PRG RAM | Super Mario Bros. 3, Kirby's Adventure, Mega Man 3 |
| 5 | MMC5 | Advanced banking, ExRAM/ExGrafix nametables, scanline IRQ, expansion audio | Castlevania III, Uncharted Waters, Just Breed |
| 7 | AxROM | 32KB PRG bank, single-screen mirroring select | Battletoads, Wizards & Warriors, Marble Madness |
| 9 | MMC2 | 8KB PRG + CHR tile-fetch latches ($FD/$FE) | Mike Tyson's Punch-Out!! |
| 10 | MMC4 | MMC2 latches with 16KB PRG banking + battery PRG RAM | Fire Emblem, Famicom Wars |
| 11 | Color Dreams | 32KB PRG / 8KB CHR with bus conflicts | Crystal Mines, Bible Adventures |
| 16 | Bandai FCG / LZ93D50 | 16KB PRG select, 1KB CHR banks, 16-bit cycle IRQ, serial-EEPROM saves | Dragon Ball Z, Rokudenashi Blues |
| 19 | Namco 163 | PRG/CHR banking, CHR-ROM nametables, wavetable expansion audio | Rolling Thunder, Megami Tensei II, Mappy Kids |
| 24 | VRC6 (VRC6a) | 16/8KB PRG, 1KB CHR, cycle/scanline IRQ, pulse+saw expansion audio | Akumajou Densetsu |
| 26 | VRC6 (VRC6b) | VRC6 on the swapped-A0/A1 pinout | Madara, Esper Dream 2 |
| 34 | BNROM / NINA-001 | 32KB PRG select (BNROM) or PRG/CHR banks + PRG RAM (NINA-001) | Deadly Towers, Impossible Mission II |
| 64 | RAMBO-1 (Tengen) | MMC3-like banking with 1KB CHR mode and a scanline/cycle IRQ | Skull & Crossbones, Klax, Rolling Thunder (Tengen) |
| 65 | Irem H3001 | 8KB PRG / 1KB CHR banking, 16-bit cycle IRQ | Daiku no Gen-san, Spartan X 2, Kaiketsu Yanchamaru 3 |
| 66 | GxROM | Combined 32KB PRG / 8KB CHR register | Super Mario Bros. + Duck Hunt, Dragon Power |
| 68 | Sunsoft-4 | 2KB CHR banks + dual CHR-ROM nametables, 16KB PRG select, battery PRG RAM | After Burner, Maharaja |
| 69 | FME-7 / Sunsoft 5B | 8KB PRG/CHR banking, cycle IRQ, 5B square-wave expansion audio | Gimmick!, Batman: Return of the Joker, Hebereke |
| 71 | Camerica / Codemasters | UxROM-like 16KB PRG select, optional single-screen mirroring | Micro Machines, Fire Hawk, Bee 52 |
| 73 | VRC3 | 16KB PRG select, CHR RAM, 16-bit cycle IRQ, PRG RAM | Salamander |
| 75 | VRC1 | 8KB PRG banks, 4KB CHR banks with a high bit in $9000 | Tetsuwan Atom, Ganbare Goemon!, Exciting Boxing |
| 159 | Bandai LZ93D50 (24C01) | Mapper 16 with a 128-byte serial EEPROM | Dragon Ball Z, Datach titles |
| 206 | Namco 108 / DxROM | MMC3's predecessor: fixed-mode banking, no IRQ/mirroring control | Gauntlet, Pac-Mania, Dragon Buster |
| 210 | Namco 175 / 340 | N163 banking without audio/IRQ; 340 adds mapper mirroring, 175 adds battery PRG RAM | Famista '92, Wagyan Land 2/3, Splatterhouse |

Expansion audio is summed into the APU mix (before decimation/filtering) for
the MMC5, VRC6, Namco 163 and Sunsoft 5B.

The Bandai FCG/LZ93D50 (16/159) serial EEPROM (24C01/24C02) is emulated as a
full I2C slave, and its contents persist to the .sav file.

The iNES four-screen flag (flags 6 bit 3) is honored on any of these boards: it
gives all four nametables their own RAM and overrides the board's mirroring,
which a handful of games — *Gauntlet*, *Rad Racer II* — depend on.
