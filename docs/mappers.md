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
| 13 | CPROM | Fixed 32KB PRG, 16KB CHR RAM with a switchable 4KB bank at $1000 | Videomation |
| 15 | K-1029 / K-1030P | Multicart: four PRG modes (32KB, UNROM, 8KB, 16KB) chosen by address bits, mirroring bit, mode-protected CHR RAM | 100-in-1 Contra Function 16 |
| 16 | Bandai FCG / LZ93D50 | 16KB PRG select, 1KB CHR banks, 16-bit cycle IRQ, serial-EEPROM saves | Dragon Ball Z, Rokudenashi Blues |
| 18 | Jaleco SS88006 | Three 8KB PRG banks, eight 1KB CHR banks (nibble registers), PRG RAM with write protect, mirroring, CPU-cycle IRQ with 4/8/12/16-bit width (uPD7756 speech not emulated) | Pizza Pop!, Ninja Jajamaru: Ginga Daisakusen, Plasma Ball |
| 19 | Namco 163 | PRG/CHR banking, CHR-ROM nametables, wavetable expansion audio | Rolling Thunder, Megami Tensei II, Mappy Kids |
| 21, 22, 23, 25 | Konami VRC2 / VRC4 | 8/16KB PRG, 1KB CHR banking, scanline IRQ; pinout variants resolved by mapper/submapper | Ganbare Goemon Gaiden 2, Crisis Force, Wai Wai World 2, TMNT (J) |
| 24 | VRC6 (VRC6a) | 16/8KB PRG, 1KB CHR, cycle/scanline IRQ, pulse+saw expansion audio | Akumajou Densetsu |
| 26 | VRC6 (VRC6b) | VRC6 on the swapped-A0/A1 pinout | Madara, Esper Dream 2 |
| 28 | Action 53 | Homebrew multicart: configurable NROM/UxROM/BNROM modes and mirroring | Action 53 homebrew collections |
| 30 | UNROM 512 | 16KB PRG select + fixed last bank, 4x8KB CHR RAM banks, header-selected H/V/one-screen/four-screen mirroring; flash writes ignored | Black Box Challenge, homebrew (RetroUSB, InfiniteNESLives) |
| 32 | Irem G101 | Two switchable 8KB PRG banks (swap mode), 1KB CHR banks, mirroring; submapper 1 fixed single-screen | Image Fight, Major League, Kaiketsu Yanchamaru |
| 33 | Taito TC0190 | Two 8KB PRG banks + two fixed, 2KB/1KB CHR banks, mirroring bit (no IRQ); auto-promotes to TC0690 if the ROM drives the IRQ registers | Don Doko Don, Insector X, Power Blazer |
| 34 | BNROM / NINA-001 | 32KB PRG select (BNROM) or PRG/CHR banks + PRG RAM (NINA-001) | Deadly Towers, Impossible Mission II |
| 37 | PAL-ZZ (MMC3 multicart) | MMC3 plus a $6000 block register picking 64/128KB PRG blocks and a 128KB CHR half | Super Mario Bros. + Tetris + Nintendo World Cup (E) |
| 38 | Bit Corp UNL-PCI556 | $7000 register: 32KB PRG + 8KB CHR | Crime Busters |
| 41 | Caltron 6-in-1 | Address-latched outer register (PRG, CHR high bits, mirroring) gating an inner CHR register | Caltron 6-in-1 |
| 46 | Rumble Station | Color Dreams banking behind a $6000 outer register | Rumble Station 15-in-1 |
| 47 | NES-QJ (MMC3 multicart) | MMC3 plus a $6000 bit selecting the 128KB PRG / CHR half | Super Spike V'Ball + Nintendo World Cup |
| 48 | Taito TC0690 | TC0190 banking plus an MMC3-style A12 scanline IRQ; mirroring at $E000 | Flintstones 2, Bubble Bobble 2, Don Doko Don 2 |
| 64 | RAMBO-1 (Tengen) | MMC3-like banking with 1KB CHR mode and a scanline/cycle IRQ | Skull & Crossbones, Klax, Rolling Thunder (Tengen) |
| 65 | Irem H3001 | 8KB PRG / 1KB CHR banking, 16-bit cycle IRQ | Daiku no Gen-san, Spartan X 2, Kaiketsu Yanchamaru 3 |
| 66 | GxROM | Combined 32KB PRG / 8KB CHR register | Super Mario Bros. + Duck Hunt, Dragon Power |
| 67 | Sunsoft-3 | 16KB PRG select, four 2KB CHR banks, mirroring, 16-bit cycle IRQ | Fantasy Zone 2, Mito Koumon, Nantettatte!! Baseball |
| 68 | Sunsoft-4 | 2KB CHR banks + dual CHR-ROM nametables, 16KB PRG select, battery PRG RAM | After Burner, Maharaja |
| 69 | FME-7 / Sunsoft 5B | 8KB PRG/CHR banking, cycle IRQ, 5B expansion audio (tones, noise, envelope) | Gimmick!, Batman: Return of the Joker, Hebereke |
| 70 | Bandai 74161 | UxROM-like 16KB PRG select + 8KB CHR select, hardwired mirroring | Kamen Rider Club, Family Trainer |
| 71 | Camerica / Codemasters | UxROM-like 16KB PRG select, optional single-screen mirroring | Micro Machines, Fire Hawk, Bee 52 |
| 72 | Jaleco JF-17 | 16KB PRG + 8KB CHR select via trigger bits (uPD7756 sound not emulated) | Pinball Quest, Moero!! Pro Tennis |
| 73 | VRC3 | 16KB PRG select, CHR RAM, 16-bit cycle IRQ, PRG RAM | Salamander |
| 74 | MMC3 + CHR RAM (Waixing) | MMC3 with CHR banks 8-9 mapped to 2KB of CHR RAM | Chinese translations of Famicom RPGs |
| 75 | VRC1 | 8KB PRG banks, 4KB CHR banks with a high bit in $9000 | Tetsuwan Atom, Ganbare Goemon!, Exciting Boxing |
| 76 | Namco 3446 | Namco 108 with four 2KB CHR banks (R2-R5), up to 128KB CHR | Digital Devil Story: Megami Tensei |
| 77 | Irem LROG017 | 32KB PRG + 2KB CHR-ROM bank, 6KB CHR RAM, four-screen nametables | Napoleon Senki |
| 78 | Holy Diver / Cosmo Carrier | 16KB PRG + 8KB CHR select, submapper-selected mirroring | Holy Diver, Uchuusen: Cosmo Carrier |
| 79, 146 | AVE NINA-03/06, Sachen 3015 | $4100 register (address & $E100): 32KB PRG + 8KB CHR | Krazy Kreatures, Deathbots, Pyramid, Metal Fighter |
| 80 | Taito X1-005 | 8KB PRG / 2KB+1KB CHR banking, mirroring, 128-byte battery RAM inside the chip | Minelvaton Saga, Kyonshiizu 2 |
| 82 | Taito X1-017 | X1-005-style banking with CHR half-swap, PRG registers shifted by 2, 5KB battery RAM in three keyed chunks | SD Keiji: Blader, Kyuukyoku Harikiri Stadium III |
| 85 | VRC7 | 8KB PRG / 1KB CHR banking, battery PRG RAM, VRC scanline/cycle IRQ, OPLL FM expansion audio (6 two-operator channels) | Lagrange Point, Tiny Toon Adventures 2 (J) |
| 86 | Jaleco JF-13 | $6000 latch: 32KB PRG + 8KB CHR (uPD7756 speech not emulated) | Moero!! Pro Yakyuu, Argus (J) |
| 87 | Jaleco J87 | $6000 latch: 8KB CHR select with swapped bits, fixed PRG | City Connection (J), The Goonies (J), Hyper Olympic |
| 88 | Namco 3433 | Namco 108 with CHR A16 tied to PPU A12 (128KB CHR) | Quinty, Dragon Spirit (J), Namco Classic |
| 89 | Sunsoft-2 (Sunsoft-3 board) | 16KB PRG + 8KB CHR select, one-screen mirroring select | Tenka no Goikenban: Mito Koumon |
| 92 | Jaleco JF-19/21 | JF-17 latch with the switchable 16KB bank at $C000 | Moero!! Pro Soccer, Moero!! Pro Yakyuu '88 |
| 93 | Sunsoft-2 (Sunsoft-3R board) | 16KB PRG select, CHR RAM | Fantasy Zone (J), Shanghai |
| 94 | UN1ROM | UxROM with the bank in bits 2-4 | Senjou no Ookami (Commando J) |
| 95 | Namco 3425 | Namco 108 whose CHR bit 5 drives the nametable select | Dragon Buster |
| 97 | Irem TAM-S1 | Last 16KB fixed at $8000, switchable 16KB at $C000, mirroring select | Kaiketsu Yanchamaru |
| 107 | Magic Dragon | One byte selects both 32KB PRG and 8KB CHR | Magic Dragon |
| 111 | GTROM / Cheapocabra | 32KB PRG banks, two 8KB CHR RAM banks, 8KB four-screen nametable RAM in two pages; flash writes ignored | Membler Industries homebrew |
| 113 | NINA-03/06 multicart (Sachen/HES) | Mapper 79 with wider PRG/CHR banks and a mirroring bit | Mind Blower Pak, Papillon Gals |
| 118 | TxSROM (MMC3) | MMC3 with CHR-bank-driven nametable select (TLSROM/TSROM) | NES Play Action Football, Armadillo |
| 119 | TQROM (MMC3) | MMC3 with 8KB CHR RAM selected by CHR bank bit 6 alongside CHR ROM | High Speed, Pin-Bot |
| 140 | Jaleco JF-11/14 | $6000 latch: 32KB PRG + 8KB CHR | Bio Senshi Dan, Mississippi Satsujin Jiken |
| 152 | Bandai 74161 | Mapper 70 banking with bit 7 one-screen mirroring select | Arkanoid II, Saint Seiya, Pocket Zaurus |
| 154 | Namco 3453 | Mapper 88 banking plus one-screen mirroring select (bit 6 of any $8000+ write) | Devil Man |
| 155 | MMC1A | MMC1 without the PRG-bank WRAM disable bit | Tatakae!! Ramen Man, Money Game |
| 158 | Tengen 800037 | RAMBO-1 with CHR-register-driven nametable select (like TxSROM) | Alien Syndrome (Tengen) |
| 159 | Bandai LZ93D50 (24C01) | Mapper 16 with a 128-byte serial EEPROM | Dragon Ball Z, Datach titles |
| 180 | UNROM (180) | UxROM variant switching only the first 16KB bank ($8000), fixed last bank | Crazy Climber |
| 184 | Sunsoft-1 | Fixed PRG, two switchable 4KB CHR banks | Atlantis no Nazo, Kanshakudama Nage Kantarou, Wing of Madoola |
| 185 | CNROM + copy protection | CHR ROM gated by a diode network; NES 2.0 submappers 4-7 or the classic value heuristic decide which latch value enables it | Spy vs Spy (J), B-Wings, Bird Week, Mighty Bomb Jack (J) |
| 191, 192, 194, 195 | MMC3 + CHR RAM (unlicensed) | MMC3 with 2-4KB of CHR RAM at specific bank numbers (bit 7 / 8-11 / 0-1 / 0-3) | Chinese translations, Sugoro Quest (C) |
| 206 | Namco 108 / DxROM | MMC3's predecessor: fixed-mode banking, no IRQ/mirroring control | Gauntlet, Pac-Mania, Dragon Buster |
| 207 | Taito X1-005 (alt. wiring) | Mapper 80 with nametables picked by CHR register bit 7 | Fudou Myouou Den, Taito Grand Prix |
| 210 | Namco 175 / 340 | N163 banking without audio/IRQ; 340 adds mapper mirroring, 175 adds battery PRG RAM | Famista '92, Wagyan Land 2/3, Splatterhouse |
| 228 | Action 52 / Cheetahmen II | Address-latched 16/32KB PRG across three 512KB chips, 8KB CHR, mirroring, four 4-bit RAM cells at $4020-$5FFF | Action 52, Cheetahmen II |
| 232 | Camerica BF9096 (Quattro) | 64KB block + 16KB bank select, block's last bank fixed; submapper 1 for the Aladdin Deck Enhancer | Quattro Adventure, Quattro Sports |
| 240 | C&E | $4020-$5FFF register: 32KB PRG + 8KB CHR, PRG RAM | Jing Ke Xin Zhuan, Sheng Huo Lie Zhuan |
| 241 | BxROM-style (Hengedianzi) | 32KB PRG select, CHR RAM, PRG RAM | Xing Ji Zheng Ba |
| 242 | Waixing (ZS) | Address-latched 32KB PRG and mirroring, PRG RAM | Wai Xing Zhan Shi |
| 246 | G0151-1 | Four 8KB PRG + four 2KB CHR registers at $6000, 2KB battery RAM, vectors from the last bank | Fong Shen Bang - Zhu Lu Zhi Zhan |

Expansion audio is summed into the APU mix (before decimation/filtering) for
the MMC5, VRC6, VRC7, Namco 163, Sunsoft 5B and the Famicom Disk System.

## Famicom Disk System

Disk images (`.fds`, with or without the 16-byte fwNES header) run on the FDS
RAM adapter core rather than an iNES mapper. It needs the 8KB FDS BIOS
(`disksys.rom`), which is not bundled. Emulated: 32KB PRG RAM + 8KB CHR RAM,
the BIOS at $E000, the CPU-cycle timer IRQ, mirroring control, the disk drive
(gapped bitstream with start marks and CRC-16, byte-transfer IRQ, motor/
end-of-head status, reads and writes), disk-side swapping with an eject delay,
and the wavetable + modulation sound channel. Disk writes persist through the
`.sav` file (the whole disk, in plain `.fds` layout). Notable games: *The
Legend of Zelda* (Disk), *Metroid* (Disk), *Akumajou Dracula*, *Doki Doki
Panic*, *Super Mario Bros. 2* (J).

The Bandai FCG/LZ93D50 (16/159) serial EEPROM (24C01/24C02) is emulated as a
full I2C slave, and its contents persist to the .sav file.

Mapper 48 (Taito TC0690) is a superset of mapper 33 (TC0190): same banking plus
a scanline IRQ, with mirroring moved from $8000 bit 6 to $E000. Both map to one
core. Standard dumps of the six TC0690 games - Flintstones (Rescue of Dino &
Hoppy), Don Doko Don 2, Bubble Bobble 2 (J), Captain Saver (J), Jetsons (J),
Bakushou!! Jinsei Gekijou 3 - carry a legacy mapper-33 header. A real TC0190 has
no registers above $BFFF, so the first CPU write to $C000-$FFFF reveals the
board as a TC0690 and the core promotes itself (enables the IRQ, switches
mirroring to $E000). This makes the mislabelled dumps boot - games that wait on
the IRQ, like Flintstones, would otherwise hang - without resorting to a
CRC/game database.

The iNES four-screen flag (flags 6 bit 3) is honored on any of these boards: it
gives all four nametables their own RAM and overrides the board's mirroring,
which a handful of games - *Gauntlet*, *Rad Racer II* - depend on.
