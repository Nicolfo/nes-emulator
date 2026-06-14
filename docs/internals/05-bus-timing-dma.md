# 4 — The bus, timing & DMA

This chapter covers the connective tissue: the CPU memory map, the clock
interleave that keeps the three chips in sync, the two kinds of DMA, and how
interrupts propagate. It is implemented mostly in [`src/bus.rs`](../../src/bus.rs)
with the DMA stall logic in [`src/cpu.rs`](../../src/cpu.rs).

## The hardware

### The CPU memory map

The 6502's 64 KB address space is carved up by address decoding:

| Range | What |
|-------|------|
| `$0000`–`$07FF` | 2 KB work RAM |
| `$0800`–`$1FFF` | Mirrors of work RAM (the RAM is only 2 KB, repeated 4×) |
| `$2000`–`$2007` | PPU registers |
| `$2008`–`$3FFF` | Mirrors of the PPU registers (every 8 bytes) |
| `$4000`–`$4013` | APU channel registers |
| `$4014` | OAM DMA trigger |
| `$4015` | APU status / channel enable |
| `$4016`–`$4017` | Controller ports (and `$4017` is also APU frame counter on write) |
| `$4020`–`$5FFF` | Cartridge expansion (mapper registers) |
| `$6000`–`$7FFF` | Cartridge PRG RAM (often battery-backed) |
| `$8000`–`$FFFF` | Cartridge PRG ROM |

Addresses from `$4020` upward are the **cartridge's** to decode — that is the
mapper's territory.

### Open bus

The NES bus is not actively driven for every address. Reading an unmapped or
write-only location returns whatever value was last floating on the bus (the
"open bus" value), which decays over time. Games occasionally rely on this, and
test ROMs definitely do.

### DMA: stealing the CPU's cycles

Two mechanisms copy data without the CPU executing instructions, by **halting**
the CPU and using its bus:

- **OAM DMA** (`$4014` write) — copies a 256-byte page of CPU RAM into the PPU's
  sprite memory in one shot. Costs 513 or 514 CPU cycles (the CPU is frozen the
  whole time). This is how games update all sprites each frame.
- **DMC DMA** — the sound chip's sample fetch (see [chapter 3](04-apu.md)). It
  steals individual cycles whenever the DMC needs its next byte.

These DMAs *halt* the CPU on a read cycle: the CPU's read keeps repeating on the
bus until the DMA engine can perform its access on the right cycle. The precise
choreography of who gets which cycle — especially when both DMAs and an
interrupt collide — is the source of the trickiest hardware behavior on the
machine.

## The implementation

### The address decoder

[`Bus::read`](../../src/bus.rs) and [`Bus::write`](../../src/bus.rs) are the
decoder. Note how each region is handled:

- RAM uses `addr & 0x07FF` to fold the mirrors.
- PPU registers use `addr & 7` and call into the PPU, **lending it the
  cartridge** (`&mut *self.cart`) because PPU memory access goes through the
  mapper.
- `$4015`, `$4016`, `$4017` have careful bit-level behavior: `$4015` is internal
  to the 2A03 and doesn't drive the external data bus (bit 5 comes from the
  CPU's `internal_bus` latch); controller reads drive only D0–D4 and leave the
  top bits as open bus.
- Cartridge ranges return `Option<u8>`; `None` means "the cartridge didn't drive
  the bus", so the bus returns the retained `open_bus` value.

`open_bus` and `internal_bus` are the two latches that make open-bus behavior
correct. `internal_bus` is the CPU's own data-bus latch, updated on real CPU
read/write cycles but *not* by a DMC fetch (because the CPU is halted then) —
this distinction is what makes a `$4015` read during a DMA return the right value.

### The per-cycle interleave

This is the mechanism that keeps the three chips locked together. One CPU cycle
is two halves:

[`tick_cycle_pre`](../../src/bus.rs):

```rust
self.cycles += 1;
self.cart.cpu_clock();          // cartridge IRQ counters / expansion audio
// ... raise a DMC DMA request if the APU needs one ...
self.apu.tick(self.cart.audio_sample());
for _ in 0..2 { self.ppu.tick(&mut *self.cart); }   // 2 of the 3 PPU dots
```

…then the CPU performs its bus access (mid-cycle)…

[`tick_cycle_post`](../../src/bus.rs):

```rust
self.ppu.tick(&mut *self.cart);   // the 3rd PPU dot
// PAL only: a 4th dot every 5th cycle (the 3.2 ratio)
// controller strobe latches on odd ("put") cycles
```

So the canonical timing — **1 CPU cycle = 3 PPU dots = 1 APU step** — is enforced
right here, and the CPU code never has to think about it: it just calls
`tick_cycle_pre` / `bus.read` / `tick_cycle_post` (wrapped in `fetch_cycle`,
see [chapter 1](02-cpu.md)).

The split into pre/post halves exists because the CPU samples the bus *mid-cycle*
(after 2 of the 3 PPU dots), and interrupt lines are polled at the *end* of the
cycle. That ordering is what lets one-dot PPU races resolve correctly against CPU
reads.

### OAM DMA

[`run_oam_dma_if_pending`](../../src/cpu.rs) runs after a `$4014` write. It
performs: one halt read, an alignment read if the cycle is odd, then 256
read-from-RAM / write-to-`$2004` pairs — exactly the 513/514 cycles hardware
takes. Crucially it ticks the bus for every one of those cycles, so the PPU keeps
rendering during the DMA.

### DMC DMA and its stalls

The DMC DMA is the most elaborate timing in the codebase, because the real chip's
behavior here is genuinely baroque. The flow:

1. The APU, inside `apu.tick`, decides it needs a sample byte and returns a
   request; `tick_cycle_pre` stores it in `bus.dmc_request` (with flags
   `dmc_ghost`, `dmc_skip_align`, `dmc_delay`).
2. On the CPU's next *read* cycle, [`rd`](../../src/cpu.rs) notices the pending
   request and calls [`dmc_dma`](../../src/cpu.rs), which halts the CPU
   (repeating its read), burns the alignment cycles to land on a "get" cycle,
   and performs the fetch, then hands the byte to the APU with `dmc_supply`.

The complications the code handles:

- **Ghost DMAs.** A request raised inside a `$4015`-disable grace window steals a
  single halt cycle and then aborts without fetching (`dmc_ghost`).
- **Blocked-retry DMAs.** A request blocked by the `$4015` enable pipeline
  retries off the normal parity grid and skips its alignment cycle
  (`dmc_skip_align`).
- **Bus conflicts.** If the halted CPU's address happens to select the APU
  register range, the DMC's fetch collides with that register — modeled as two
  reads on the same cycle in `dmc_dma`, each given its real hardware semantics.
- **DMC during OAM DMA.** A DMC fetch that comes due while an OAM DMA is running
  shares the OAM DMA's halt/dummy cycles and steals a "get" cycle in the middle;
  `run_oam_dma_if_pending` tracks this with the `dmc_track!` macro and
  `dmc_served` counter.

The constants `DMC_LOAD_DELAY` and `DMC_GET_PARITY` at the top of `bus.rs` pin
down this alignment (change them and rebuild to experiment).

> **Why so much effort?** None of this matters for *running* games visibly, but
> a handful of games and the AccuracyCoin suite depend on the CPU being delayed
> by exactly the right number of cycles at exactly the right moments, because a
> DMA landing one cycle early or late shifts a sprite-zero hit or a scroll split.

### Interrupts on the bus

The bus exposes two line-level queries the CPU polls every cycle:

- [`nmi_line`](../../src/bus.rs) — the PPU's NMI output (vblank flag AND NMI
  enable). The CPU edge-detects this (see [chapter 1](02-cpu.md)).
- [`irq_asserted`](../../src/bus.rs) — the logical OR of the APU's IRQ (frame
  counter + DMC) and the cartridge's IRQ. Level-triggered; the CPU honors it only
  when its `I` flag is clear.

Because these are just level queries, the timing-sensitive races (NMI
suppression, the exact cycle an IRQ is recognized) are decided by *when* the CPU
polls, not by any push/queue mechanism — which is why they come out right.

### Controllers on the bus

A `$4016` write sets the controller strobe; reads of `$4016`/`$4017` shift one
button bit out at a time. The strobe latch and shift happen on specific cycle
parities, driven from `tick_cycle_post` calling
[`Controller::clock_put_cycle`](../../src/controller.rs). See
[chapter 6](07-frontend.md) for the controller itself.

### Where to look

| You want to understand… | Look at |
|---|---|
| The CPU memory map | `Bus::read`, `Bus::write` |
| Open bus | `open_bus`, `internal_bus`, the `Option<u8>` cartridge returns |
| The 3:1 clock interleave | `tick_cycle_pre`, `tick_cycle_post` |
| OAM DMA | `run_oam_dma_if_pending` (cpu.rs) |
| DMC DMA stalls | `rd`, `wr`, `dmc_dma` (cpu.rs); the request raise in `tick_cycle_pre` |
| Interrupt lines | `nmi_line`, `irq_asserted` |
