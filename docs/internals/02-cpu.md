# 1 — The CPU (6502)

## The hardware

The NES CPU is a **6502** core (specifically the 2A03's version, with BCD
decimal mode disabled). It is the workhorse: it runs the game's logic, reads the
controllers, and orchestrates the PPU and APU by writing to their registers.

### Registers

The 6502 has a famously small register set:

| Register | Size | Purpose |
|----------|------|---------|
| `A` | 8-bit | Accumulator — the target of most arithmetic/logic |
| `X`, `Y` | 8-bit | Index registers, used for addressing and counting |
| `SP` | 8-bit | Stack pointer — the stack lives at `$0100`–`$01FF` |
| `PC` | 16-bit | Program counter |
| `P` | 8-bit | Status flags |

The status flags in `P` are: **C**arry, **Z**ero, **I**nterrupt-disable,
**D**ecimal (inert on NES), **B**reak, **U**nused (always 1), o**V**erflow, and
**N**egative. In the code these are the constants `C, Z, I, D, B, U, V, N` at the
top of [`src/cpu.rs`](../../src/cpu.rs).

### Memory and addressing

The 6502 sees a flat 64 KB. A few regions are special by convention:

- **Zero page** (`$0000`–`$00FF`) — addressable with a single byte, so
  instructions that use it are shorter and faster. Games keep hot variables here.
- **Stack** (`$0100`–`$01FF`) — `SP` indexes into this page; it grows downward.
- **Vectors** at the very top: `$FFFA/B` = NMI handler, `$FFFC/D` = reset,
  `$FFFE/F` = IRQ/BRK handler.

An instruction is an opcode byte plus 0–2 operand bytes. The *addressing mode*
decides how the operand bytes turn into the address the instruction operates on:
immediate, zero-page, absolute, indexed (add `X`/`Y`), indirect, and so on.

### Cycles, not just instructions

Every 6502 instruction takes a fixed, known number of clock cycles, and — this
is the key — **the CPU performs exactly one memory access per cycle** (a read or
a write; there is no cycle with no bus activity). Even "wasted" cycles do a
*dummy* read or write to some address. Real hardware behavior, and many game
tricks, depend on exactly *which* addresses get touched on *which* cycles,
including those dummy accesses (e.g. an indexed read that crosses a page boundary
does an extra dummy read at the wrong address first).

### Interrupts

Three things can divert the CPU:

- **RESET** — at power-on, jumps to the address in the reset vector.
- **NMI** (non-maskable interrupt) — the PPU pulses this once per frame at the
  start of vblank (if enabled). Games use it as their per-frame heartbeat. It is
  **edge-triggered**: it fires on the rising edge of the PPU's NMI line.
- **IRQ** — requested by the APU frame counter, the DMC, or a cartridge mapper.
  It is **level-triggered** and is ignored while the `I` flag is set.

The 6502 decides whether to service an interrupt by *polling* these lines near
the end of each instruction. The exact cycle on which it polls produces several
subtle, game-visible timing behaviors (see Quirks below).

## The implementation

The CPU is a **cycle-accurate, cycle-stepped** core
([`src/cpu.rs`](../../src/cpu.rs)). It does **not** use a cycle-count lookup
table. Instead, each instruction is written as the exact sequence of bus accesses
the real chip performs, and the cycle count emerges from counting those accesses.

### The cycle primitives

Everything bottoms out in a handful of helpers that each represent *one CPU
cycle*:

- [`fetch_cycle`](../../src/cpu.rs) — the atom: bump the cycle counter, tick the
  bus's "pre" half (APU ×1, PPU ×2), perform the bus read, tick the "post" half
  (PPU ×1), then poll the interrupt lines. This is where one CPU cycle = 3 PPU
  dots is enforced.
- [`read_cycle`](../../src/cpu.rs) — a `fetch_cycle` that also latches the value
  onto the CPU's internal data bus (used by open-bus behavior).
- [`write_cycle`](../../src/cpu.rs) — the write equivalent.
- [`rd`](../../src/cpu.rs) / [`wr`](../../src/cpu.rs) — wrappers around
  read/write that additionally handle a DMC DMA stealing the cycle (see
  [chapter 4](05-bus-timing-dma.md)).

Because the bus is ticked *inside* each of these, the PPU and APU stay perfectly
aligned to the CPU with no separate scheduler.

### The step function

[`Cpu::step`](../../src/cpu.rs) executes one instruction (or one interrupt
sequence):

1. If an interrupt was decided last time, run the 7-cycle
   [`interrupt_sequence`](../../src/cpu.rs).
2. Otherwise fetch the opcode with `fetch8` and dispatch via
   [`exec`](../../src/cpu.rs), a giant `match` over all 256 opcodes.
3. Run a pending OAM DMA if the instruction was a `$4014` write.
4. Decide whether the *next* step services an interrupt, from the poll state.

### Addressing modes

The `Mode` enum and [`addr`](../../src/cpu.rs) turn an addressing mode into the
effective address while performing the correct cycle-by-cycle accesses. The
crucial detail is the `is_read` parameter and the page-cross handling in
[`indexed`](../../src/cpu.rs):

```rust
let crossed = (base & 0xFF00) != (a & 0xFF00);
if !is_read || crossed {
    self.rd((base & 0xFF00) | (a & 0x00FF)); // the dummy read at the wrong page
}
```

- A **read** instruction only does the extra dummy read when the index actually
  crosses a page (that is the famous "+1 cycle on page cross").
- A **store** or **read-modify-write** *always* does the uncorrected dummy read,
  even when no page is crossed. Getting this right is required to pass hardware
  tests and to emulate certain mappers that watch the bus.

### Instruction bodies

The arithmetic/logic operations are small helpers: `adc_val`, `asl_val`,
`rol_val`, `compare`, etc., each updating flags via `set_zn`/`set_flag`.
Read-modify-write instructions go through [`rmw`](../../src/cpu.rs), which
faithfully does **read → dummy write-back of the original value → write the
result** — three accesses, because that is what the hardware does (and some
mappers latch on that dummy write).

Branches ([`branch`](../../src/cpu.rs)) reproduce the taken-branch dummy read and
the second dummy read when the branch target crosses a page.

#### Official and unofficial opcodes

The `match` in [`exec`](../../src/cpu.rs) covers **every** opcode value,
including the ~80 "unofficial"/"illegal" instructions that games and test ROMs
actually use: `LAX`, `SAX`, `DCP`, `ISC`, `SLO`, `RLA`, `SRE`, `RRA`, the
immediate oddballs `ANC/ALR/ARR/AXS/XAA/LXA`, the unstable
`SHA/SHX/SHY/TAS/LAS`, and the `KIL/JAM` opcodes that lock the CPU. Most are
"combine two official ops", so they reuse the same value helpers.

### Interrupts and the poll pipeline

This is the most intricate part of the CPU and the reason it can pass
cycle-exact tests.

- [`poll_lines`](../../src/cpu.rs) runs at the end of *every* cycle. It detects
  the **NMI rising edge** (`line && !nmi_line_prev`), and computes whether an
  interrupt is currently wanted (`nmi_pending || (irq && I clear)`).
- The result is kept in a two-stage pipeline `poll_cur` → `poll_prev`. The
  decision to take an interrupt on the *next* instruction is read from
  `poll_prev`, i.e. **the poll result as of the second-to-last cycle of the
  instruction that just finished**. That one-cycle offset is exactly what real
  6502 silicon does, and it is what produces the correct latency for `CLI`,
  `SEI`, and `PLP` (the flag change is "too late" to affect the interrupt about
  to be taken) and the taken-branch interrupt delay.

The header comment on `src/cpu.rs` summarizes the philosophy; the fields
`nmi_pending`, `poll_prev`, `poll_cur`, `take_interrupt`, and `suppress_poll`
implement it.

> **Quirk — NMI hijacking.** During the 7-cycle interrupt sequence (and `BRK`),
> the vector to jump through is chosen *late*, after the status byte is pushed.
> So if an NMI edge arrives mid-sequence while an IRQ/BRK is being serviced, the
> CPU ends up jumping through the NMI vector instead. See the vector selection in
> [`interrupt_sequence`](../../src/cpu.rs) and opcode `0x00` (BRK).

> **Quirk — NMI/$2002 race.** Because NMI is an edge detect on the PPU's actual
> NMI line, a `$2002` read that lands on the exact dot the vblank flag would set
> can suppress the flag and thus the whole NMI. This is handled on the PPU side
> (`suppress_vbl`, see [chapter 2](03-ppu.md)) and falls out naturally here
> because the CPU only ever looks at the line level.

> **Quirk — the SHA/SHX/SHY "& (H+1)" loss.** These unstable stores normally AND
> the stored value with the target's high address byte + 1. If a DMC DMA stalled
> the cycle right before the write, that AND is skipped. The flag `dmc_stalled`
> threads this through [`sh_write`](../../src/cpu.rs).

### Reset

[`Cpu::reset`](../../src/cpu.rs) loads `PC` from the reset vector
(`$FFFC/$FFFD`), sets `SP = 0xFD` and `P = I|U`, and burns the 7 cycles the
hardware takes — by calling `bus.tick_cycle()` seven times so the PPU/APU advance
correctly during reset too.

### Where to look

| You want to understand… | Look at |
|---|---|
| One CPU cycle | `fetch_cycle`, `read_cycle`, `write_cycle` |
| The instruction loop | `step`, `exec` |
| Addressing + page-cross dummies | `addr`, `indexed` |
| Read-modify-write timing | `rmw` |
| Interrupt timing | `poll_lines`, `interrupt_sequence`, `step`'s tail |
| DMA stalls | `rd`, `wr`, `dmc_dma`, `run_oam_dma_if_pending` (see ch. 4) |
