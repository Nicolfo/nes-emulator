# Accuracy testing

The emulator passes all **140 / 140** tests of
[AccuracyCoin](https://github.com/100thCoin/AccuracyCoin), a hardware-verified NES
accuracy test ROM covering CPU timing and unofficial opcodes, open bus, DMA
(OAM + DMC, including aborted 1-cycle DMAs and bus conflicts), the APU frame
counter/DMC, controller strobing, and dot-level PPU behavior (sprite evaluation,
OAM corruption, stale shift registers, the $2007 data-bus state machine).

It also passes `nestest` with cycle-exact logging (see `tests/nestest.rs`).

## Running the AccuracyCoin suite

The ROM is not redistributed here; download the prebuilt binary from the
AccuracyCoin repository into the project root:

```
curl -L -o AccuracyCoin.nes https://github.com/100thCoin/AccuracyCoin/raw/main/AccuracyCoin.nes
```

Then either run the integration test (skips itself when the ROM is absent):

```
cargo test --release --test accuracycoin_rom
```

or use the interactive harness, which can run single tests and print diagnostics:

```
cargo run --release --example accuracy_rom                      # run all 140 tests
cargo run --release --example accuracy_rom 12 8 479             # one test: page, row, result addr
cargo run --release --example accuracy_rom -- --skip 11,12      # skip whole pages
cargo run --release --example accuracy_rom -- --markskip 13:6   # skip individual tests
```

### Debug environment variables

The cycle/DMA/PPU trace logging is compiled out by default so release builds
carry no instrumentation cost in the hot paths. Build with the `trace` feature to
enable it, then set the relevant variable at runtime:

```
cargo run --features trace --example accuracy_rom        # logging available
NES_DMA_LOG=1 cargo run --features trace -- path\to\rom.nes
```

Without `--features trace` these variables are ignored.

| Variable | Effect |
|---|---|
| `NES_DMA_LOG=1` | log every DMC DMA event (raise/halt/fetch/ghost) and $4015 write |
| `NES_EXEC_TRACE=1` | log executed instructions with PC < $0800 or in $4000-$401F, plus interrupt sequences |
| `NES_EXEC_WINDOW=a:b` | log all instructions in that CPU-cycle range |
| `NES_NMI_LOG=1` | log NMI edges |
| `NES_VRAM_WATCH=hhh` | log writes to CIRAM offset `hhh` (hex) |
| `NES_MMC5_LOG=1`, `NES_MMC5_SL=1` | log MMC5 register access / scanline-IRQ counter |
| `DUMP_RANGE=addr:len` | (harness) dump RAM after a single test finishes, e.g. `DUMP_RANGE=500:96` (always available) |

### Timing model constants

These values match AccuracyCoin's hardware-derived answer keys for this
emulator's CPU/PPU alignment. They are plain source constants (not runtime
knobs); change them only for timing experiments and rebuild.

| Constant | Value | Location | Meaning |
|---|---|---|---|
| `MASK_DELAY` | 4 | `src/ppu.rs` | PPU dots before a $2001 write takes effect |
| `PPUDATA_READ_DELAY` | 5 | `src/ppu.rs` | PPU dots before a $2007 read's buffer refill fires (data-bus state machine) |
| `DMC_LOAD_DELAY` | 1 | `src/bus.rs` | CPU cycles of RDY delay for a DMC "load" DMA |
| `DMC_GET_PARITY` | 1 | `src/bus.rs` | bus-cycle parity on which DMC fetches land |
