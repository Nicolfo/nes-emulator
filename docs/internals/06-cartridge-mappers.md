# 5 — Cartridges & the `Mapper` trait

Per the project's request, this chapter explains **what the mapper abstraction
is and what the trait does**, not the internals of each individual mapper. For
the full list of supported mappers and the games they cover, see
[docs/mappers.md](../mappers.md).

## The hardware

### Why mappers exist

The CPU can only see PRG ROM in the `$8000`–`$FFFF` window (32 KB) and the PPU
can only see CHR in `$0000`–`$1FFF` (8 KB). But games quickly grew far larger
than that. The solution: put a small chip on the cartridge — a **mapper** — that
sits between the console and the ROM and decides, at any moment, *which* slice of
the cartridge's (much larger) ROM is currently visible in those windows.

A game switches banks by **writing to addresses in ROM space**. There's no RAM
there to write to; the write is a signal the mapper chip intercepts. For example,
writing a bank number to `$8000` might tell the mapper "map bank 5 into
`$8000`–`$BFFF` now." The next instruction the CPU fetches from `$8000` comes
from different physical ROM than it did a moment ago.

Mappers range from trivial (NROM: no banking at all) to elaborate (MMC3, MMC5,
VRC6) with scanline-counting IRQ generators, extra RAM, custom nametable
arrangements, and even **expansion sound channels**. They also control
**mirroring** — how the four logical nametables fold onto the console's 2 KB of
video RAM (or onto cartridge RAM).

### iNES — the ROM file format

A `.nes` file is a 16-byte header followed by the PRG ROM then the CHR ROM. The
header records: the size of each ROM in banks, the **mapper number**, the default
**mirroring**, whether the cartridge has **battery-backed RAM** (save games), an
optional trainer, and (in the newer NES 2.0 variant) the target **region**.

## The implementation

### Loading a ROM

[`load_rom`](../../src/cartridge.rs) parses the iNES/NES 2.0 header:

- Validates the `NES\x1A` magic.
- Computes the mapper number from the split nibbles in flags 6 and 7.
- Detects the region (NES 2.0 timing byte, with a fallback to the legacy
  TV-system bit) → `Region::Ntsc` / `Region::Pal`.
- Reads the default mirroring and the battery flag.
- Slices out the PRG and CHR byte ranges.
- Constructs the right concrete mapper based on the mapper number and returns it
  as a `Box<dyn Mapper>`, along with the region and battery flag.

That `match mapper_id { 0 => Nrom, 1 => Mmc1, ... }` is the only place that knows
about specific mapper types; everything else in the emulator talks to the
abstract trait.

### The `Mapper` trait — the contract

The whole point of the abstraction is that the CPU bus and the PPU don't care
*which* cartridge is plugged in — they just call trait methods. The trait lives
in [`src/mapper.rs`](../../src/mapper.rs). Here is what each method is *for*:

```rust
pub trait Mapper {
    fn cpu_read(&mut self, addr: u16) -> u8;          // CPU reads PRG space ($8000+ etc.)
    fn cpu_write(&mut self, addr: u16, val: u8);      // CPU writes PRG space → bank-switch signal
    fn ppu_read(&mut self, addr: u16) -> u8;          // PPU reads CHR ($0000–$1FFF)
    fn ppu_write(&mut self, addr: u16, val: u8);      // PPU writes CHR (CHR RAM carts)
    fn mirroring(&self) -> Mirroring;                 // current nametable arrangement
    // ...defaulted methods below...
}
```

The two `cpu_*` / `ppu_*` pairs are the core: they are how the two buses reach
the cartridge. A `cpu_write` into ROM space is *not* a memory store — it is the
game poking the mapper's registers to change banking, which is why the method
takes the address and value and the mapper interprets them however its hardware
would.

Everything else has a **default implementation** so simple mappers (like NROM)
only implement the five core methods:

| Method | Default | Purpose |
|--------|---------|---------|
| `prg_ram_read` | `None` | Serve `$6000`–`$7FFF` PRG RAM; `None` = open bus |
| `prg_ram` / `prg_ram_mut` | `None` | Raw RAM access for battery `.sav` save/restore |
| `irq` | `false` | The cartridge's IRQ line level (for mappers with IRQ counters) |
| `cpu_clock` | no-op | Called once per CPU cycle — clocks cartridge IRQ counters and expansion audio |
| `audio_sample` | `0.0` | The mapper's expansion-audio output this cycle, summed into the APU mix |
| `cpu_reg_read` | `None` | Readable registers in `$4020`–`$5FFF` |
| `nt_target` | mirror via `mirroring()` | **Where a nametable access is routed** (see below) |
| `cpu_bus_write` | no-op | Observe CPU writes to PPU registers (MMC5 snoops these) |

#### Nametable routing: `nt_target` and `NtTarget`

This is the one piece worth understanding in detail because it shows how the
trait stays general. When the PPU accesses a nametable address, it asks the
mapper [`nt_target`](../../src/mapper.rs), which returns an `NtTarget`:

- `NtTarget::Ciram(offset)` — "use the console's 2 KB video RAM at this offset."
  The default implementation computes that offset from the cartridge's mirroring
  via [`mirror_nt`](../../src/mapper.rs), which handles the four standard modes
  (horizontal, vertical, single-screen low/high).
- `NtTarget::Cart` — "the cartridge will serve/accept this byte itself" (used by
  mappers that map CHR ROM or their own RAM into nametable space, e.g. N163,
  MMC5 fill mode).

Because every nametable fetch goes through this call, mappers can also use it
just to *observe* the PPU address bus — which is how A12-based scanline counters
(MMC3) tick.

### How the rest of the emulator uses the trait

- The **bus** holds `cart: Box<dyn Mapper>` and forwards CPU accesses to it
  (`cpu_read`, `cpu_write`, `prg_ram_read`, `cpu_reg_read`), calls `cpu_clock`
  once per cycle, samples `audio_sample`, and queries `irq` (see
  [chapter 4](05-bus-timing-dma.md)).
- The **PPU** is handed `&mut dyn Mapper` on every `tick` and calls `ppu_read`,
  `ppu_write`, and `nt_target` for its fetches (see [chapter 2](03-ppu.md)).
- The **cartridge loader** is the only code that names concrete mapper types.

### Mirroring

The [`Mirroring`](../../src/mapper.rs) enum and `mirror_nt` are shared
infrastructure rather than mapper-specific. Mirroring controls whether scrolling
wraps horizontally or vertically (or shows a single screen), and many mappers can
change it at runtime by returning a different `mirroring()`.

### Battery saves

When a cartridge declares a battery, [`Nes::battery_ram`](../../src/nes.rs) /
`load_battery_ram` route through `prg_ram` / `prg_ram_mut` so the frontend can
persist `$6000`–`$7FFF` to a `.sav` file beside the ROM (see
[chapter 6](07-frontend.md)). This is why those two raw-access methods exist
separately from `prg_ram_read` — they bypass any enable/banking logic.

### Where to look

| You want to understand… | Look at |
|---|---|
| ROM parsing + mapper selection | `load_rom` (cartridge.rs) |
| The abstraction contract | `trait Mapper` (mapper.rs) |
| Nametable routing | `NtTarget`, `nt_target`, `mirror_nt` |
| The simplest concrete mapper | `src/mapper/nrom.rs` |
| The full mapper list | [docs/mappers.md](../mappers.md) |
