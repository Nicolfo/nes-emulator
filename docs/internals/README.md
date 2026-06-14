# NES Internals — how the console works and how this emulator implements it

This is a deep-dive guide to the Nintendo Entertainment System (NES) and to the
way this Rust codebase emulates it. It is meant to be read top to bottom, but
each chapter also stands on its own.

Every chapter has the same two-part shape:

1. **The hardware** — what the real chip does and why.
2. **The implementation** — how `src/` mirrors that, with pointers to the exact
   functions and lines.

## Reading order

| # | Chapter | What it covers |
|---|---------|----------------|
| 0 | [Architecture overview](01-architecture.md) | The chips, how they connect, the master clock, and how the source tree maps onto the hardware |
| 1 | [The CPU (6502)](02-cpu.md) | The 2A03's 6502 core, cycle-stepped execution, addressing modes, interrupts |
| 2 | [The PPU (picture)](03-ppu.md) | Scanline/dot rendering, the loopy scroll registers, background + sprite pipelines |
| 3 | [The APU (sound)](04-apu.md) | The five sound channels, the frame counter, the mixer and resampling |
| 4 | [The bus, timing & DMA](05-bus-timing-dma.md) | The memory map, the 3-dots-per-cycle interleave, OAM/DMC DMA, interrupts |
| 5 | [Cartridges & the Mapper trait](06-cartridge-mappers.md) | iNES parsing, region detection, and what the `Mapper` trait is for |
| 6 | [The frontend](07-frontend.md) | The `Nes` facade, the windowing/audio/input host, frame pacing |

## The one-paragraph summary

A NES is four cooperating parts on a shared clock: a **CPU** (a 6502 variant
that runs the game code), a **PPU** that paints the picture one dot at a time, an
**APU** that generates sound, and a **cartridge** that supplies the program and
graphics ROM (plus, via its *mapper* chip, the ability to swap which slices of
that ROM are visible). They communicate through a 16-bit **address bus**. This
emulator models all of them at the granularity of a single CPU cycle: each cycle
the CPU performs exactly one memory access, and the bus advances the PPU by three
dots and the APU by one step in lockstep. Because the timing is modeled at this
fine a grain, the quirks that real games depend on — mid-frame scroll changes,
sprite-zero hits, DMA stalls, interrupt-timing races — emerge naturally instead
of being special-cased.

## A note on accuracy

This emulator is unusually precise: it passes all 140 tests of the
hardware-verified [AccuracyCoin](https://github.com/100thCoin/AccuracyCoin) suite
and the cycle-exact `nestest` log (see [docs/accuracy.md](../accuracy.md)). That
is why the code contains so many comments about one-cycle races and "ghost"
DMAs. Throughout these chapters, the genuinely esoteric details are flagged as
**Quirk** notes — you can skip them on a first read and still come away
understanding the machine.
