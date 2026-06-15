# 0 - Architecture overview

## The hardware

The NES (released as the Famicom in Japan, 1983) is built around three custom
chips and a cartridge slot.

### The chips

- **2A03 (NTSC) / 2A07 (PAL) - the CPU package.** Inside this single chip are
  actually two things: a **6502** processor core (minus the 6502's decimal/BCD
  arithmetic mode, which Nintendo fused off) and the **APU**, the audio hardware.
  The 6502 runs the game's program. It is an 8-bit CPU with a 16-bit address bus,
  so it can address 64 KB. It runs at **1.789773 MHz** on NTSC machines.
- **2C02 - the PPU (Picture Processing Unit).** This is the graphics chip. It has
  its *own* separate 16 KB address space (distinct from the CPU's) wired to the
  cartridge's graphics ROM and to 2 KB of on-board video RAM. It generates a
  256×240 picture by walking across the screen one pixel ("dot") at a time, in
  step with how a CRT television's electron beam sweeps. It runs at three times
  the CPU clock: **5.369318 MHz**, i.e. 3 PPU dots per CPU cycle.
- **The cartridge.** Not just passive ROM. It carries program ROM ("PRG"),
  graphics ROM ("CHR"), optionally some RAM (sometimes battery-backed for save
  games), and very often a **mapper** - a small chip that lets the game switch
  which banks of its ROM are mapped into the limited address windows the CPU and
  PPU can see. Mappers are how games grew far larger than the 6502's 64 KB reach.

### Two separate address spaces

This is the single most important structural fact about the NES:

- The **CPU bus** (`$0000`–`$FFFF`) sees: work RAM, the PPU's eight registers,
  the APU/controller registers, and the cartridge's PRG ROM/RAM.
- The **PPU bus** (`$0000`–`$3FFF`) sees: the cartridge's CHR (pattern) memory,
  the nametables (background layout), and the palette.

The CPU cannot read video memory directly. It pokes the PPU's registers to ask
the PPU to do reads and writes on its behalf. This indirection - and the fact
that it can only safely happen while the PPU is *not* drawing - shapes how every
NES game is structured.

### The master clock and regions

Everything derives from one crystal. On NTSC, the CPU gets one cycle for every 3
PPU dots; the APU is clocked at the CPU rate. PAL machines use a different
crystal and a different ratio (3.2 PPU dots per CPU cycle), a taller frame, and
slightly different sound-channel timing. A game's cartridge header declares which
region it targets.

```
            crystal
               │
      ┌────────┼─────────┐
   ÷ 12       (PPU)    (APU = CPU rate)
      │         │
   6502 CPU   2C02 PPU
  1.79 MHz   5.37 MHz   →  3 dots : 1 cycle (NTSC)
```

## The implementation

The source tree maps almost one-to-one onto the hardware:

| Hardware | Source file | Type |
|----------|-------------|------|
| 6502 core | `src/cpu.rs` | `Cpu` |
| PPU (2C02) | `src/ppu.rs` | `Ppu` |
| APU (part of 2A03) | `src/apu.rs` | `Apu` |
| CPU bus / address decoding | `src/bus.rs` | `Bus` |
| Cartridge + mapper chip | `src/cartridge.rs`, `src/mapper.rs`, `src/mapper/*` | `Mapper` trait |
| Controller | `src/controller.rs` | `Controller` |
| Palette → RGB table | `src/palette.rs` | `NES_PALETTE` |
| Whole-console facade | `src/nes.rs` | `Nes` |
| Savestate snapshot format | `src/savestate.rs` | `SaveState` |

### How they own each other

The ownership graph follows the wiring. The CPU owns the bus; the bus owns
everything the CPU can reach:

```
Nes                       (src/nes.rs)
└── Cpu                    (src/cpu.rs)   registers + the interrupt pipeline
    └── Bus                (src/bus.rs)   CPU address map + the per-cycle tick
        ├── ram: [u8; 2048]               2 KB work RAM
        ├── Ppu            (src/ppu.rs)
        ├── Apu            (src/apu.rs)
        ├── Controller     (src/controller.rs)
        └── cart: Box<dyn Mapper>          (src/mapper/*)
```

You can see this in [`Cpu::new`](../../src/cpu.rs) taking a `Bus`, and
[`Bus::with_region`](../../src/bus.rs) constructing the `Ppu`, `Apu`,
`Controller`, and holding the `Box<dyn Mapper>`. The PPU and cartridge each need
to talk to one another (the PPU fetches graphics through the mapper), so the
mapper is *not* owned by the PPU; instead every PPU operation that touches the
cartridge takes `&mut dyn Mapper` as an argument. That is why you will see
signatures like `ppu.tick(&mut *self.cart)` all over `bus.rs` - the bus lends the
cartridge to the PPU for the duration of a call.

### The master clock, in code

The 3-dots-per-cycle relationship lives in `Bus`. A single CPU cycle is split
into two halves around the moment the CPU samples the bus
([`tick_cycle_pre`](../../src/bus.rs) and
[`tick_cycle_post`](../../src/bus.rs)):

- `tick_cycle_pre` clocks the APU once and the PPU twice, then the CPU does its
  read/write.
- `tick_cycle_post` clocks the PPU a third time. On PAL, a `pal_phase` counter
  adds a fourth dot on every fifth cycle, realizing the 3.2 ratio.

So "the bus runs the rest of the console while the CPU thinks" - the CPU never
calls the PPU or APU directly for timing; it just performs bus cycles, and the
bus fans those cycles out to the other chips. This is the backbone of the whole
emulator and is covered in detail in [chapter 4](05-bus-timing-dma.md).

### The emulation loop

At the very top, [`Nes::run_frame`](../../src/nes.rs) just steps the CPU until
the PPU signals it has finished a frame:

```rust
pub fn run_frame(&mut self) {
    self.cpu.bus.ppu.frame_complete = false;
    while !self.cpu.bus.ppu.frame_complete {
        self.cpu.step();          // one instruction; the bus ticks PPU+APU inside
    }
}
```

`frame_complete` is set by the PPU when it reaches the start of vblank (scanline
241, dot 1). One `cpu.step()` executes one full instruction, and *inside* that
step the bus advances the PPU and APU cycle-by-cycle. The host frontend
(`src/main.rs`) calls `run_frame` ~60 times a second and presents the resulting
framebuffer; see [chapter 6](07-frontend.md).

### Why cycle-stepping matters

A simpler emulator might execute a whole instruction, look up "this instruction
takes 4 cycles", then advance the PPU by 12 dots in one lump. That works for many
games but breaks the ones that change PPU state *in the middle* of an
instruction's memory accesses - which is common, because games race the beam.
This emulator instead advances the PPU/APU between every individual bus access,
so timing-dependent effects fall out for free. The trade-off is that the CPU is
written as explicit per-cycle access sequences rather than a cycle-count table;
that is the subject of the next chapter.
