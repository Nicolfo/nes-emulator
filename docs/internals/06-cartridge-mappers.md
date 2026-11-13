# 5 - Cartridges & the `Mapper` trait

Per the project's request, this chapter explains **what the mapper abstraction
is and what the trait does**, not the internals of each individual mapper. For
the full list of supported mappers and the games they cover, see
[docs/mappers.md](../mappers.md).

## The hardware

### Why mappers exist

The CPU can only see PRG ROM in the `$8000`–`$FFFF` window (32 KB) and the PPU
can only see CHR in `$0000`–`$1FFF` (8 KB). But games quickly grew far larger
than that. The solution: put a small chip on the cartridge - a **mapper** - that
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
**mirroring** - how the four logical nametables fold onto the console's 2 KB of
video RAM (or onto cartridge RAM).

### iNES - the ROM file format

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
- Reads the default mirroring (horizontal/vertical, or four-screen when the
  flags 6 bit-3 four-screen pad is set) and the battery flag.
- Computes the PRG and CHR ROM sizes via `rom_size`. NES 2.0 widens the iNES
  bank counts with a 4-bit MSB (header byte 9) and adds an exponent-multiplier
  encoding (`2^E * (2*M+1)` bytes) for sizes that aren't a whole number of
  banks; plain iNES uses the byte-4/5 counts directly. Getting this right
  matters for large (>4 MB PRG / >2 MB CHR) NES 2.0 images, whose CHR would
  otherwise start at the wrong file offset.
- Slices out the PRG and CHR byte ranges.
- Constructs the right concrete mapper based on the mapper number and returns it
  as a `Box<dyn Mapper>`, along with the region and battery flag.
- Applies the NES 2.0 RAM-size fields (header bytes 10/11, `64 << n` shift
  counts, volatile + battery nibbles summed) via `Mapper::set_ram_sizes`, so
  boards with more than the 8 KB default (SOROM/SXROM work RAM, 32 KB CHR RAM
  carts) get the allocation the game expects. Sizes round up to a whole 8 KB;
  0 (and plain iNES, which has no such fields) keeps each board's default.
- If the header's trainer flag is set, copies the 512-byte trainer into PRG RAM
  at `$7000`-`$71FF` (via `prg_ram_mut`) after the mapper is built - a no-op for
  boards with no PRG RAM.

That `match mapper_id { 0 => Nrom, 1 => Mmc1, ... }` is the only place that knows
about specific mapper types; everything else in the emulator talks to the
abstract trait.

### The `Mapper` trait - the contract

The whole point of the abstraction is that the CPU bus and the PPU don't care
*which* cartridge is plugged in - they just call trait methods. The trait lives
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
the cartridge. A `cpu_write` into ROM space is *not* a memory store - it is the
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
| `cpu_clock` | no-op | Called once per CPU cycle - clocks cartridge IRQ counters and expansion audio |
| `audio_sample` | `0.0` | The mapper's expansion-audio output this cycle, summed into the APU mix |
| `cpu_reg_read` | `None` | Readable registers in `$4020`–`$5FFF` |
| `nt_target` | mirror via `mirroring()` | **Where a nametable access is routed** (see below) |
| `cpu_bus_write` | no-op | Observe CPU writes to PPU registers (MMC5 snoops these) |

#### Nametable routing: `nt_target` and `NtTarget`

This is the one piece worth understanding in detail because it shows how the
trait stays general. When the PPU accesses a nametable address, it asks the
mapper [`nt_target`](../../src/mapper.rs), which returns an `NtTarget`:

- `NtTarget::Ciram(offset)` - "use the nametable RAM at this offset." The
  default implementation computes that offset from the cartridge's mirroring
  via [`mirror_nt`](../../src/mapper.rs), which handles horizontal, vertical,
  single-screen low/high, and four-screen. The first four modes index the
  console's 2 KB CIRAM; four-screen spans the full 4 KB (CIRAM plus the
  cartridge's extra 2 KB), so the PPU's nametable RAM is sized to 4 KB.
- `NtTarget::Cart` - "the cartridge will serve/accept this byte itself" (used by
  mappers that map CHR ROM or their own RAM into nametable space, e.g. N163,
  MMC5 fill mode).

Because every nametable fetch goes through this call, mappers can also use it
just to *observe* the PPU address bus - which is how A12-based scanline counters
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

`Mirroring::FourScreen` is the exception: it is fixed by the cartridge's
four-screen pad (iNES flags 6 bit 3), gives all four nametables their own 1 KB
of RAM, and overrides any mapper mirroring register - so a board that normally
drives mirroring from a register (MMC3, etc.) leaves it alone once four-screen
is in effect. Games such as *Gauntlet* and *Rad Racer II* rely on this.

### Battery saves

When a cartridge declares a battery, [`Nes::battery_ram`](../../src/nes.rs) /
`load_battery_ram` route through `prg_ram` / `prg_ram_mut` so the frontend can
persist `$6000`–`$7FFF` to a `.sav` file beside the ROM (see
[chapter 6](07-frontend.md)). This is why those two raw-access methods exist
separately from `prg_ram_read` - they bypass any enable/banking logic.

### Famicom Disk System

The FDS is not an iNES board: [`Nes::new_fds`](../../src/nes.rs) takes a disk
image (fwNES `FDS` header or headerless, 65500 bytes per side) plus the
8 KB `disksys.rom` BIOS and builds an [`Fds`](../../src/mapper/fds.rs) mapper
directly, bypassing `load_rom`. The RAM adapter maps 32 KB of PRG RAM at
`$6000`–`$DFFF`, the BIOS at `$E000`, 8 KB of CHR RAM, a CPU-cycle timer IRQ
(`$4020`–`$4022`), mirroring via `$4025` bit 3, and the wavetable/FM
expansion sound (`FdsAudio` in `fds_audio.rs`, `$4040`–`$4092`). `$4087` bit 6
forces the mod accumulator's carry, stepping the mod table on every
16-cycle unit tick regardless of the mod frequency.

The drive reads a per-side *gapped stream* rebuilt from the `.fds` data: a
28300-bit leading gap, then per block a `$80` start mark, the block, its
CRC-16 (poly `$8408`) and a 976-bit gap - the sequence the BIOS synchronises
on. The head advances one byte every 149 CPU cycles while the motor runs and
raises the byte-transfer IRQ/flag; writes land two bytes behind the read
position and fold back into the `.fds` layout when the write session ends.
That layout is what `prg_ram` exposes, so disk saves persist through the same
`.sav` path as battery RAM (`battery_ram_loaded` rebuilds the streams after a
restore). Side changes go through `Nes::fds_insert`, which leaves the drive
empty for about a second so the BIOS notices the swap.

### Vs. System

Nintendo's Vs. UniSystem arcade boards are NES hardware with an RGB PPU and
coin-op I/O ([`src/vs.rs`](../../src/vs.rs)). `vs::detect` reads the header:
NES 2.0 console type 1 with byte 13 naming the PPU, or the plain iNES flags7
bit 0, which is trusted only when bytes 12-15 are clean and defaults to the
RP2C03 since iNES can't name the PPU. Any mapper can be a Vs. game (1, 4, 206
and others); mapper 99 is the dedicated discrete board.

- **Palette.** `Ppu::set_vs_ppu` swaps the composite NES palette for the
  chip's colour ROM: 2C03 / RC2C03B, or one of the four scrambled RP2C04
  orders (canonical ROM + per-variant lookup table, data from Nintendulator).
  On these RGB PPUs the $2001 emphasis bits force a channel to full.
- **RC2C05.** $2000/$2001 are swapped and $2002's low bits return the chip
  ID ($1B, $3D or $1C).
- **I/O.** `Bus::vs` adds the service button (bit 2), DIP 1-2 (bits 3-4) and
  coins (bits 5-6) to $4016 reads, and DIP 3-8 to $4017 bits 2-7. A $4016
  write also reaches `Mapper::vs_io_write`, where mapper 99 takes its bank
  bit. The host drives them via `Nes::set_vs_inputs` / `Nes::set_vs_dip`.

Not emulated: the Vs. DualSystem (two linked boards), and the RBI Baseball,
TKO Boxing and Super Xevious protection chips.

### Where to look

| You want to understand… | Look at |
|---|---|
| ROM parsing + mapper selection | `load_rom` (cartridge.rs) |
| The abstraction contract | `trait Mapper` (mapper.rs) |
| Nametable routing | `NtTarget`, `nt_target`, `mirror_nt` |
| The simplest concrete mapper | `src/mapper/nrom.rs` |
| The full mapper list | [docs/mappers.md](../mappers.md) |
| Famicom Disk System (drive, timer, audio) | `src/mapper/fds.rs`, `src/mapper/fds_audio.rs` |
| Vs. System (palettes, coin I/O, mapper 99) | `src/vs.rs`, `src/mapper/vs_unisystem.rs` |
