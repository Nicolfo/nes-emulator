//! Cycle-accurate 6502 core.
//!
//! Every CPU cycle performs exactly one bus access (read or write), and the
//! bus (PPU x3, APU x1) is ticked once per access. Instruction cycle counts
//! therefore fall out of the per-cycle access sequences instead of a lookup
//! table. Interrupts are polled every cycle; the decision to service one is
//! taken from the state at the end of the second-to-last cycle of the
//! preceding instruction, which reproduces the I-flag latency of CLI/SEI/PLP
//! and the taken-branch interrupt delay. NMI is an edge detect on the PPU's
//! NMI line, so $2002 reads racing the VBlank flag suppress it naturally.

use crate::bus::Bus;

pub const C: u8 = 0x01;
pub const Z: u8 = 0x02;
pub const I: u8 = 0x04;
pub const D: u8 = 0x08;
pub const B: u8 = 0x10;
pub const U: u8 = 0x20;
pub const V: u8 = 0x40;
pub const N: u8 = 0x80;

#[derive(Clone, Copy)]
enum Mode {
    Imm,
    Zp,
    ZpX,
    ZpY,
    Abs,
    AbsX,
    AbsY,
    IndX,
    IndY,
}

pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub p: u8,
    pub cycles: u64,
    pub bus: Bus,

    nmi_line_prev: bool,
    nmi_pending: bool,
    // Interrupt poll pipeline: poll_prev is the poll result as of the end of
    // the second-to-last cycle of the instruction that just finished.
    poll_prev: bool,
    poll_cur: bool,
    take_interrupt: bool,
    // BRK and the interrupt sequence don't poll on their final cycles, so the
    // first instruction of the handler always executes before a pending
    // interrupt is serviced.
    suppress_poll: bool,
    in_oam_dma: bool,
    // Set when the last rd() was stalled by a DMC DMA (SHA/SHX/SHY/TAS lose
    // the & (H+1) on the stored value when the DMA lands before their write).
    dmc_stalled: bool,
}

impl Cpu {
    pub fn new(bus: Bus) -> Self {
        Cpu {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            pc: 0,
            p: I | U,
            cycles: 0,
            bus,
            nmi_line_prev: false,
            nmi_pending: false,
            poll_prev: false,
            poll_cur: false,
            take_interrupt: false,
            suppress_poll: false,
            in_oam_dma: false,
            dmc_stalled: false,
        }
    }

    pub fn reset(&mut self) {
        let lo = self.bus.read(0xFFFC) as u16;
        let hi = self.bus.read(0xFFFD) as u16;
        self.pc = (hi << 8) | lo;
        self.sp = 0xFD;
        self.p = I | U;
        self.cycles = 7;
        for _ in 0..7 {
            self.bus.tick_cycle();
        }
    }

    pub fn step(&mut self) -> u64 {
        let start = self.cycles;
        if self.take_interrupt {
            crate::trace_log!(
                "NES_EXEC_TRACE",
                "cyc {} IRQ/NMI seq at pc={:04X}",
                self.cycles,
                self.pc
            );
            self.interrupt_sequence();
        } else {
            let _pc = self.pc;
            let op = self.fetch8();
            #[cfg(feature = "trace")]
            {
                let in_window = std::env::var("NES_EXEC_WINDOW").ok().is_some_and(|w| {
                    w.split_once(':').is_some_and(|(a, b)| {
                        a.parse::<u64>().is_ok_and(|a| self.cycles >= a)
                            && b.parse::<u64>().is_ok_and(|b| self.cycles <= b)
                    })
                });
                if in_window
                    || std::env::var("NES_EXEC_TRACE").is_ok()
                        && (_pc < 0x0800 || (0x4000..=0x401F).contains(&_pc))
                {
                    eprintln!(
                        "cyc {} EXEC {:04X} op={:02X} a={:02X} x={:02X} sp={:02X} p={:02X}",
                        self.cycles, _pc, op, self.a, self.x, self.sp, self.p
                    );
                }
            }
            self.exec(op);
            self.run_oam_dma_if_pending();
        }
        // Decide whether the *next* step services an interrupt, based on the
        // poll state at the penultimate cycle of what just executed.
        let suppressed = std::mem::take(&mut self.suppress_poll);
        self.take_interrupt = self.poll_prev && !suppressed;
        self.cycles - start
    }

    // ---- cycle primitives ----

    fn poll_lines(&mut self) {
        let line = self.bus.nmi_line();
        if line && !self.nmi_line_prev {
            self.nmi_pending = true;
            crate::trace_log!(
                "NES_NMI_LOG",
                "cyc {} NMI edge at pc={:04X}",
                self.cycles,
                self.pc
            );
        }
        self.nmi_line_prev = line;
        self.poll_prev = self.poll_cur;
        self.poll_cur = self.nmi_pending || (self.bus.irq_asserted() && self.p & I == 0);
    }

    /// One read cycle: the access samples mid-cycle (after 2 of 3 PPU dots);
    /// the interrupt lines are polled at the end of the cycle.
    fn read_cycle(&mut self, addr: u16) -> u8 {
        let v = self.fetch_cycle(addr);
        self.bus.internal_bus = v;
        v
    }

    /// A DMA engine's sample-fetch read cycle: drives the external bus but
    /// leaves the halted CPU's internal data bus latch untouched.
    fn fetch_cycle(&mut self, addr: u16) -> u8 {
        self.cycles += 1;
        self.bus.tick_cycle_pre();
        let v = self.bus.read(addr);
        self.bus.tick_cycle_post();
        self.poll_lines();
        v
    }

    /// One write cycle.
    fn write_cycle(&mut self, addr: u16, val: u8) {
        self.cycles += 1;
        self.bus.tick_cycle_pre();
        self.bus.internal_bus = val;
        self.bus.write(addr, val);
        self.bus.tick_cycle_post();
        self.poll_lines();
    }

    /// CPU read with DMC DMA stall handling: the DMC halts the CPU on a read
    /// cycle, and the halted cycles repeat the same read on the bus.
    fn rd(&mut self, addr: u16) -> u8 {
        self.dmc_stalled = false;
        if self.bus.dmc_request.is_some() && self.bus.dmc_delay == 0 && !self.in_oam_dma {
            if self.bus.dmc_ghost {
                // Aborted DMA: one halt cycle, no fetch.
                self.bus.dmc_request = None;
                self.bus.dmc_ghost = false;
                self.bus.apu.dmc_abort_fetch();
                crate::trace_log!(
                    "NES_DMA_LOG",
                    "cyc {} GHOST halt at {:04X}",
                    self.cycles,
                    addr
                );
                self.read_cycle(addr);
            } else {
                self.dmc_dma(addr);
            }
            self.dmc_stalled = true;
        }
        self.read_cycle(addr)
    }

    fn wr(&mut self, addr: u16, val: u8) {
        // A ghost (1-cycle aborted) DMA landing on a CPU write cycle does not
        // occur at all: writes ignore RDY, and the abort consumes the request.
        if self.bus.dmc_request.is_some()
            && self.bus.dmc_ghost
            && self.bus.dmc_delay == 0
            && !self.in_oam_dma
        {
            self.bus.dmc_request = None;
            self.bus.dmc_ghost = false;
            self.bus.apu.dmc_abort_fetch();
            crate::trace_log!(
                "NES_DMA_LOG",
                "cyc {} GHOST dropped on write at {:04X}",
                self.cycles,
                addr
            );
        }
        self.write_cycle(addr, val);
    }

    /// DMC sample fetch. The CPU is halted on its read cycle: the read keeps
    /// occurring on the bus (with side effects) until the fetch executes on a
    /// "get" cycle. If the CPU's halted address selects the APU registers,
    /// the fetch suffers a bus conflict with the register at the mirrored
    /// offset.
    fn dmc_dma(&mut self, addr: u16) {
        let Some(sample_addr) = self.bus.dmc_request.take() else {
            return;
        };
        crate::trace_log!(
            "NES_DMA_LOG",
            "cyc {} HALT at addr {:04X}",
            self.cycles,
            addr
        );
        // A reload DMA steals 3-4 cycles: halt, dummy, an alignment cycle if
        // the next cycle is not a "get" cycle, then the fetch on a get cycle.
        // The halted CPU repeats its read (with side effects) every cycle.
        // A blocked-attempt retry already consumed its alignment waiting for
        // the enable pipeline: halt, put, get (3 cycles, off-parity fetch).
        let skip_align = std::mem::take(&mut self.bus.dmc_skip_align);
        let parity = crate::bus::DMC_GET_PARITY;
        self.read_cycle(addr); // halt
        self.read_cycle(addr); // dummy
        if !skip_align {
            while (self.cycles + 1) & 1 != parity {
                self.read_cycle(addr); // alignment
            }
        }
        let v = if (0x4000..=0x401F).contains(&addr) {
            // Bus conflict: the halted CPU's address keeps the APU registers
            // active (mirrored every $20 across the address space), so the
            // register at the sample address's mirror responds together with
            // the ROM. Model as two reads on the same cycle; bus.read already
            // gives each register its hardware semantics ($4015 not driving
            // the data bus, $4016/$4017 driving D0-D4 only, others open bus).
            self.cycles += 1;
            self.bus.tick_cycle_pre();
            let _sample = self.bus.read(sample_addr);
            let v = self.bus.read(0x4000 | (sample_addr & 0x1F));
            self.bus.tick_cycle_post();
            self.poll_lines();
            v
        } else {
            self.fetch_cycle(sample_addr)
        };
        crate::trace_log!("NES_DMA_LOG", "cyc {} FETCH", self.cycles);
        self.bus.apu.dmc_supply(v);
    }

    /// OAM DMA: runs after the STA $4014 write cycle completes. 513/514 cycles:
    /// one halt read (at the address the CPU would have fetched next), an
    /// alignment read if on an odd cycle, then 256 read/write pairs through
    /// $2004.
    fn run_oam_dma_if_pending(&mut self) {
        let Some(page) = self.bus.oam_dma_page.take() else {
            return;
        };
        self.in_oam_dma = true;
        let halt_addr = self.pc;
        let apu_active = (0x4000..=0x401F).contains(&halt_addr);
        // A DMC DMA arriving while OAM DMA runs shares its halt and dummy
        // cycles with the OAM DMA (which keeps going), then steals a "get"
        // cycle for its fetch, after which the OAM DMA needs one alignment
        // cycle. Track how many halt/dummy cycles a pending DMC request has
        // already been served; the fetch may steal a get once it has two.
        let mut dmc_pending = self.bus.dmc_request.is_some();
        let mut dmc_served: u8 = 0;
        macro_rules! dmc_track {
            () => {
                if self.bus.dmc_request.is_some() {
                    if self.bus.dmc_ghost {
                        // An aborted DMA's halt overlaps the OAM DMA's cycle.
                        self.bus.dmc_request = None;
                        self.bus.dmc_ghost = false;
                        self.bus.apu.dmc_abort_fetch();
                    } else if !dmc_pending {
                        dmc_pending = true;
                        dmc_served = 0;
                    } else if dmc_served < 3 {
                        dmc_served += 1;
                    }
                }
            };
        }
        self.read_cycle(halt_addr);
        dmc_track!();
        if self.cycles & 1 == 1 {
            self.read_cycle(halt_addr);
            dmc_track!();
        }
        let base = (page as u16) << 8;
        for i in 0..256u16 {
            // OAM read happens on a get cycle; a ripe DMC fetch steals it,
            // then the OAM DMA runs one alignment cycle before resuming.
            if dmc_pending && dmc_served >= 2 && self.bus.dmc_request.is_some() {
                let sample_addr = self.bus.dmc_request.take().unwrap();
                let v = self.fetch_cycle(sample_addr);
                self.bus.apu.dmc_supply(v);
                dmc_pending = false;
                self.dma_read_cycle(base + i, apu_active); // alignment
                dmc_track!();
            }
            let v = self.dma_read_cycle(base + i, apu_active);
            dmc_track!();
            self.write_cycle(0x2004, v);
            dmc_track!();
        }
        // A DMC DMA still pending when the OAM DMA ends carries over the
        // halt/dummy cycles it was already served.
        if dmc_pending && self.bus.dmc_request.is_some() {
            let sample_addr = self.bus.dmc_request.take().unwrap();
            while dmc_served < 2 {
                self.read_cycle(halt_addr);
                dmc_served += 1;
            }
            let parity = crate::bus::DMC_GET_PARITY;
            while (self.cycles + 1) & 1 != parity {
                self.read_cycle(halt_addr); // alignment
            }
            let v = self.fetch_cycle(sample_addr);
            self.bus.apu.dmc_supply(v);
        }
        self.in_oam_dma = false;
    }

    /// One OAM-DMA read cycle (APU register range gated by the CPU address bus).
    fn dma_read_cycle(&mut self, addr: u16, apu_active: bool) -> u8 {
        self.cycles += 1;
        self.bus.tick_cycle_pre();
        let v = self.bus.read_for_dma(addr, apu_active);
        self.bus.tick_cycle_post();
        self.poll_lines();
        v
    }

    // ---- interrupt sequences ----

    fn interrupt_sequence(&mut self) {
        // 7 cycles: two dummy fetches, three pushes, two vector reads.
        self.rd(self.pc);
        self.rd(self.pc);
        self.push(((self.pc) >> 8) as u8);
        self.push(self.pc as u8);
        // Vector selection happens late, so an NMI can hijack an IRQ.
        let vector = if self.nmi_pending {
            self.nmi_pending = false;
            0xFFFA
        } else {
            0xFFFE
        };
        self.push((self.p & !B) | U);
        self.p |= I;
        let lo = self.rd(vector) as u16;
        let hi = self.rd(vector + 1) as u16;
        self.pc = (hi << 8) | lo;
        self.suppress_poll = true;
    }

    // ---- memory helpers ----

    fn fetch8(&mut self) -> u8 {
        let v = self.rd(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    fn fetch16(&mut self) -> u16 {
        let lo = self.fetch8() as u16;
        let hi = self.fetch8() as u16;
        (hi << 8) | lo
    }

    fn push(&mut self, v: u8) {
        self.wr(0x0100 + self.sp as u16, v);
        self.sp = self.sp.wrapping_sub(1);
    }

    fn pop(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.rd(0x0100 + self.sp as u16)
    }

    /// Dummy read of the next opcode byte (implied/accumulator instructions).
    fn implied_dummy(&mut self) {
        self.rd(self.pc);
    }

    // ---- addressing ----
    // `index_penalty`: read instructions only re-read on a page cross;
    // stores and RMWs always perform the uncorrected dummy read.

    fn addr(&mut self, m: Mode, is_read: bool) -> u16 {
        match m {
            Mode::Imm => {
                let a = self.pc;
                self.pc = self.pc.wrapping_add(1);
                a
            }
            Mode::Zp => self.fetch8() as u16,
            Mode::ZpX => {
                let zp = self.fetch8();
                self.rd(zp as u16); // dummy read at unindexed address
                zp.wrapping_add(self.x) as u16
            }
            Mode::ZpY => {
                let zp = self.fetch8();
                self.rd(zp as u16);
                zp.wrapping_add(self.y) as u16
            }
            Mode::Abs => self.fetch16(),
            Mode::AbsX => self.indexed(self.x, is_read),
            Mode::AbsY => self.indexed(self.y, is_read),
            Mode::IndX => {
                let zp = self.fetch8();
                self.rd(zp as u16); // dummy read at pointer base
                let zp = zp.wrapping_add(self.x);
                let lo = self.rd(zp as u16) as u16;
                let hi = self.rd(zp.wrapping_add(1) as u16) as u16;
                (hi << 8) | lo
            }
            Mode::IndY => {
                let zp = self.fetch8();
                let lo = self.rd(zp as u16) as u16;
                let hi = self.rd(zp.wrapping_add(1) as u16) as u16;
                let base = (hi << 8) | lo;
                let a = base.wrapping_add(self.y as u16);
                let crossed = (base & 0xFF00) != (a & 0xFF00);
                if !is_read || crossed {
                    self.rd((base & 0xFF00) | (a & 0x00FF));
                }
                a
            }
        }
    }

    fn indexed(&mut self, index: u8, is_read: bool) -> u16 {
        let base = self.fetch16();
        let a = base.wrapping_add(index as u16);
        let crossed = (base & 0xFF00) != (a & 0xFF00);
        if !is_read || crossed {
            self.rd((base & 0xFF00) | (a & 0x00FF));
        }
        a
    }

    fn set_zn(&mut self, v: u8) {
        self.p = (self.p & !(Z | N)) | if v == 0 { Z } else { 0 } | (v & N);
    }

    fn set_flag(&mut self, flag: u8, on: bool) {
        if on {
            self.p |= flag;
        } else {
            self.p &= !flag;
        }
    }

    // ---- instruction bodies ----

    fn load(&mut self, m: Mode) -> u8 {
        let a = self.addr(m, true);
        self.rd(a)
    }

    fn lda(&mut self, m: Mode) {
        let v = self.load(m);
        self.a = v;
        self.set_zn(v);
    }

    fn ldx(&mut self, m: Mode) {
        let v = self.load(m);
        self.x = v;
        self.set_zn(v);
    }

    fn ldy(&mut self, m: Mode) {
        let v = self.load(m);
        self.y = v;
        self.set_zn(v);
    }

    fn sta(&mut self, m: Mode) {
        let a = self.addr(m, false);
        self.wr(a, self.a);
    }

    fn stx(&mut self, m: Mode) {
        let a = self.addr(m, false);
        self.wr(a, self.x);
    }

    fn sty(&mut self, m: Mode) {
        let a = self.addr(m, false);
        self.wr(a, self.y);
    }

    fn and(&mut self, m: Mode) {
        let v = self.load(m);
        self.a &= v;
        self.set_zn(self.a);
    }

    fn ora(&mut self, m: Mode) {
        let v = self.load(m);
        self.a |= v;
        self.set_zn(self.a);
    }

    fn eor(&mut self, m: Mode) {
        let v = self.load(m);
        self.a ^= v;
        self.set_zn(self.a);
    }

    fn bit(&mut self, m: Mode) {
        let v = self.load(m);
        self.set_flag(Z, self.a & v == 0);
        self.set_flag(N, v & 0x80 != 0);
        self.set_flag(V, v & 0x40 != 0);
    }

    fn adc_val(&mut self, v: u8) {
        let sum = self.a as u16 + v as u16 + (self.p & C) as u16;
        let r = sum as u8;
        self.set_flag(C, sum > 0xFF);
        self.set_flag(V, (!(self.a ^ v) & (self.a ^ r) & 0x80) != 0);
        self.a = r;
        self.set_zn(r);
    }

    fn adc(&mut self, m: Mode) {
        let v = self.load(m);
        self.adc_val(v);
    }

    fn sbc(&mut self, m: Mode) {
        let v = self.load(m);
        self.adc_val(v ^ 0xFF);
    }

    fn compare(&mut self, m: Mode, reg: u8) {
        let v = self.load(m);
        self.set_flag(C, reg >= v);
        self.set_zn(reg.wrapping_sub(v));
    }

    fn asl_val(&mut self, v: u8) -> u8 {
        self.set_flag(C, v & 0x80 != 0);
        let r = v << 1;
        self.set_zn(r);
        r
    }

    fn lsr_val(&mut self, v: u8) -> u8 {
        self.set_flag(C, v & 1 != 0);
        let r = v >> 1;
        self.set_zn(r);
        r
    }

    fn rol_val(&mut self, v: u8) -> u8 {
        let carry_in = self.p & C;
        self.set_flag(C, v & 0x80 != 0);
        let r = (v << 1) | carry_in;
        self.set_zn(r);
        r
    }

    fn ror_val(&mut self, v: u8) -> u8 {
        let carry_in = (self.p & C) << 7;
        self.set_flag(C, v & 1 != 0);
        let r = (v >> 1) | carry_in;
        self.set_zn(r);
        r
    }

    /// Read-modify-write: read, dummy write-back of the original, final write.
    fn rmw(&mut self, m: Mode, f: fn(&mut Cpu, u8) -> u8) {
        let a = self.addr(m, false);
        let v = self.rd(a);
        self.wr(a, v);
        let r = f(self, v);
        self.wr(a, r);
    }

    fn branch(&mut self, cond: bool) {
        let off = self.fetch8() as i8;
        if cond {
            // Dummy read of the next opcode at the un-branched PC.
            self.rd(self.pc);
            let old = self.pc;
            let target = old.wrapping_add(off as u16);
            if (old & 0xFF00) != (target & 0xFF00) {
                // PCH not fixed yet: dummy read at (old page | new PCL).
                self.pc = (old & 0xFF00) | (target & 0x00FF);
                self.rd(self.pc);
            }
            self.pc = target;
        }
    }

    fn nop_read(&mut self, m: Mode, is_read: bool) {
        let a = self.addr(m, is_read);
        let _ = self.rd(a);
    }

    // ---- unofficial instruction bodies ----

    fn lax(&mut self, m: Mode) {
        let v = self.load(m);
        self.a = v;
        self.x = v;
        self.set_zn(v);
    }

    fn sax(&mut self, m: Mode) {
        let a = self.addr(m, false);
        self.wr(a, self.a & self.x);
    }

    fn dcp(&mut self, m: Mode) {
        let a = self.addr(m, false);
        let original = self.rd(a);
        self.wr(a, original);
        let v = original.wrapping_sub(1);
        self.wr(a, v);
        self.set_flag(C, self.a >= v);
        self.set_zn(self.a.wrapping_sub(v));
    }

    fn isc(&mut self, m: Mode) {
        let a = self.addr(m, false);
        let original = self.rd(a);
        self.wr(a, original);
        let v = original.wrapping_add(1);
        self.wr(a, v);
        self.adc_val(v ^ 0xFF);
    }

    fn slo(&mut self, m: Mode) {
        let a = self.addr(m, false);
        let original = self.rd(a);
        self.wr(a, original);
        let r = self.asl_val(original);
        self.wr(a, r);
        self.a |= r;
        self.set_zn(self.a);
    }

    fn rla(&mut self, m: Mode) {
        let a = self.addr(m, false);
        let original = self.rd(a);
        self.wr(a, original);
        let r = self.rol_val(original);
        self.wr(a, r);
        self.a &= r;
        self.set_zn(self.a);
    }

    fn sre(&mut self, m: Mode) {
        let a = self.addr(m, false);
        let original = self.rd(a);
        self.wr(a, original);
        let r = self.lsr_val(original);
        self.wr(a, r);
        self.a ^= r;
        self.set_zn(self.a);
    }

    fn rra(&mut self, m: Mode) {
        let a = self.addr(m, false);
        let original = self.rd(a);
        self.wr(a, original);
        let r = self.ror_val(original);
        self.wr(a, r);
        self.adc_val(r);
    }

    /// SHA/SHX/SHY/TAS core: cycles are fetch lo, fetch hi, dummy read at the
    /// uncorrected address, then the write of val & (base high byte + 1). On a
    /// page cross the corrupted value also replaces the target's high byte.
    fn sh_write(&mut self, base: u16, index: u8, val: u8) {
        let target = base.wrapping_add(index as u16);
        self.rd((base & 0xFF00) | (target & 0x00FF)); // dummy read
        // RDY quirk: if a DMA stalled the cycle before the write, the value
        // is not ANDed with the high byte of the address.
        let v = if self.dmc_stalled {
            val
        } else {
            val & ((base >> 8) as u8).wrapping_add(1)
        };
        let addr = if (base & 0xFF00) != (target & 0xFF00) {
            ((v as u16) << 8) | (target & 0x00FF)
        } else {
            target
        };
        self.wr(addr, v);
    }

    fn exec(&mut self, op: u8) {
        match op {
            // LDA
            0xA9 => self.lda(Mode::Imm),
            0xA5 => self.lda(Mode::Zp),
            0xB5 => self.lda(Mode::ZpX),
            0xAD => self.lda(Mode::Abs),
            0xBD => self.lda(Mode::AbsX),
            0xB9 => self.lda(Mode::AbsY),
            0xA1 => self.lda(Mode::IndX),
            0xB1 => self.lda(Mode::IndY),
            // LDX
            0xA2 => self.ldx(Mode::Imm),
            0xA6 => self.ldx(Mode::Zp),
            0xB6 => self.ldx(Mode::ZpY),
            0xAE => self.ldx(Mode::Abs),
            0xBE => self.ldx(Mode::AbsY),
            // LDY
            0xA0 => self.ldy(Mode::Imm),
            0xA4 => self.ldy(Mode::Zp),
            0xB4 => self.ldy(Mode::ZpX),
            0xAC => self.ldy(Mode::Abs),
            0xBC => self.ldy(Mode::AbsX),
            // STA
            0x85 => self.sta(Mode::Zp),
            0x95 => self.sta(Mode::ZpX),
            0x8D => self.sta(Mode::Abs),
            0x9D => self.sta(Mode::AbsX),
            0x99 => self.sta(Mode::AbsY),
            0x81 => self.sta(Mode::IndX),
            0x91 => self.sta(Mode::IndY),
            // STX / STY
            0x86 => self.stx(Mode::Zp),
            0x96 => self.stx(Mode::ZpY),
            0x8E => self.stx(Mode::Abs),
            0x84 => self.sty(Mode::Zp),
            0x94 => self.sty(Mode::ZpX),
            0x8C => self.sty(Mode::Abs),
            // Transfers
            0xAA => {
                self.implied_dummy();
                self.x = self.a;
                self.set_zn(self.x);
            }
            0xA8 => {
                self.implied_dummy();
                self.y = self.a;
                self.set_zn(self.y);
            }
            0x8A => {
                self.implied_dummy();
                self.a = self.x;
                self.set_zn(self.a);
            }
            0x98 => {
                self.implied_dummy();
                self.a = self.y;
                self.set_zn(self.a);
            }
            0xBA => {
                self.implied_dummy();
                self.x = self.sp;
                self.set_zn(self.x);
            }
            0x9A => {
                self.implied_dummy();
                self.sp = self.x;
            }
            // Stack
            0x48 => {
                self.implied_dummy();
                self.push(self.a);
            }
            0x08 => {
                self.implied_dummy();
                self.push(self.p | B | U);
            }
            0x68 => {
                self.implied_dummy();
                self.rd(0x0100 + self.sp as u16); // dummy stack read
                self.a = self.pop();
                self.set_zn(self.a);
            }
            0x28 => {
                self.implied_dummy();
                self.rd(0x0100 + self.sp as u16);
                let v = self.pop();
                self.p = (v & !B) | U;
            }
            // AND
            0x29 => self.and(Mode::Imm),
            0x25 => self.and(Mode::Zp),
            0x35 => self.and(Mode::ZpX),
            0x2D => self.and(Mode::Abs),
            0x3D => self.and(Mode::AbsX),
            0x39 => self.and(Mode::AbsY),
            0x21 => self.and(Mode::IndX),
            0x31 => self.and(Mode::IndY),
            // ORA
            0x09 => self.ora(Mode::Imm),
            0x05 => self.ora(Mode::Zp),
            0x15 => self.ora(Mode::ZpX),
            0x0D => self.ora(Mode::Abs),
            0x1D => self.ora(Mode::AbsX),
            0x19 => self.ora(Mode::AbsY),
            0x01 => self.ora(Mode::IndX),
            0x11 => self.ora(Mode::IndY),
            // EOR
            0x49 => self.eor(Mode::Imm),
            0x45 => self.eor(Mode::Zp),
            0x55 => self.eor(Mode::ZpX),
            0x4D => self.eor(Mode::Abs),
            0x5D => self.eor(Mode::AbsX),
            0x59 => self.eor(Mode::AbsY),
            0x41 => self.eor(Mode::IndX),
            0x51 => self.eor(Mode::IndY),
            // BIT
            0x24 => self.bit(Mode::Zp),
            0x2C => self.bit(Mode::Abs),
            // ADC
            0x69 => self.adc(Mode::Imm),
            0x65 => self.adc(Mode::Zp),
            0x75 => self.adc(Mode::ZpX),
            0x6D => self.adc(Mode::Abs),
            0x7D => self.adc(Mode::AbsX),
            0x79 => self.adc(Mode::AbsY),
            0x61 => self.adc(Mode::IndX),
            0x71 => self.adc(Mode::IndY),
            // SBC
            0xE9 | 0xEB => self.sbc(Mode::Imm),
            0xE5 => self.sbc(Mode::Zp),
            0xF5 => self.sbc(Mode::ZpX),
            0xED => self.sbc(Mode::Abs),
            0xFD => self.sbc(Mode::AbsX),
            0xF9 => self.sbc(Mode::AbsY),
            0xE1 => self.sbc(Mode::IndX),
            0xF1 => self.sbc(Mode::IndY),
            // CMP
            0xC9 => self.compare(Mode::Imm, self.a),
            0xC5 => self.compare(Mode::Zp, self.a),
            0xD5 => self.compare(Mode::ZpX, self.a),
            0xCD => self.compare(Mode::Abs, self.a),
            0xDD => self.compare(Mode::AbsX, self.a),
            0xD9 => self.compare(Mode::AbsY, self.a),
            0xC1 => self.compare(Mode::IndX, self.a),
            0xD1 => self.compare(Mode::IndY, self.a),
            // CPX / CPY
            0xE0 => self.compare(Mode::Imm, self.x),
            0xE4 => self.compare(Mode::Zp, self.x),
            0xEC => self.compare(Mode::Abs, self.x),
            0xC0 => self.compare(Mode::Imm, self.y),
            0xC4 => self.compare(Mode::Zp, self.y),
            0xCC => self.compare(Mode::Abs, self.y),
            // INC / DEC memory
            0xE6 => self.rmw(Mode::Zp, |c, v| {
                let r = v.wrapping_add(1);
                c.set_zn(r);
                r
            }),
            0xF6 => self.rmw(Mode::ZpX, |c, v| {
                let r = v.wrapping_add(1);
                c.set_zn(r);
                r
            }),
            0xEE => self.rmw(Mode::Abs, |c, v| {
                let r = v.wrapping_add(1);
                c.set_zn(r);
                r
            }),
            0xFE => self.rmw(Mode::AbsX, |c, v| {
                let r = v.wrapping_add(1);
                c.set_zn(r);
                r
            }),
            0xC6 => self.rmw(Mode::Zp, |c, v| {
                let r = v.wrapping_sub(1);
                c.set_zn(r);
                r
            }),
            0xD6 => self.rmw(Mode::ZpX, |c, v| {
                let r = v.wrapping_sub(1);
                c.set_zn(r);
                r
            }),
            0xCE => self.rmw(Mode::Abs, |c, v| {
                let r = v.wrapping_sub(1);
                c.set_zn(r);
                r
            }),
            0xDE => self.rmw(Mode::AbsX, |c, v| {
                let r = v.wrapping_sub(1);
                c.set_zn(r);
                r
            }),
            // INX/INY/DEX/DEY
            0xE8 => {
                self.implied_dummy();
                self.x = self.x.wrapping_add(1);
                self.set_zn(self.x);
            }
            0xC8 => {
                self.implied_dummy();
                self.y = self.y.wrapping_add(1);
                self.set_zn(self.y);
            }
            0xCA => {
                self.implied_dummy();
                self.x = self.x.wrapping_sub(1);
                self.set_zn(self.x);
            }
            0x88 => {
                self.implied_dummy();
                self.y = self.y.wrapping_sub(1);
                self.set_zn(self.y);
            }
            // Shifts: accumulator
            0x0A => {
                self.implied_dummy();
                let v = self.a;
                self.a = self.asl_val(v);
            }
            0x4A => {
                self.implied_dummy();
                let v = self.a;
                self.a = self.lsr_val(v);
            }
            0x2A => {
                self.implied_dummy();
                let v = self.a;
                self.a = self.rol_val(v);
            }
            0x6A => {
                self.implied_dummy();
                let v = self.a;
                self.a = self.ror_val(v);
            }
            // Shifts: memory
            0x06 => self.rmw(Mode::Zp, Cpu::asl_val),
            0x16 => self.rmw(Mode::ZpX, Cpu::asl_val),
            0x0E => self.rmw(Mode::Abs, Cpu::asl_val),
            0x1E => self.rmw(Mode::AbsX, Cpu::asl_val),
            0x46 => self.rmw(Mode::Zp, Cpu::lsr_val),
            0x56 => self.rmw(Mode::ZpX, Cpu::lsr_val),
            0x4E => self.rmw(Mode::Abs, Cpu::lsr_val),
            0x5E => self.rmw(Mode::AbsX, Cpu::lsr_val),
            0x26 => self.rmw(Mode::Zp, Cpu::rol_val),
            0x36 => self.rmw(Mode::ZpX, Cpu::rol_val),
            0x2E => self.rmw(Mode::Abs, Cpu::rol_val),
            0x3E => self.rmw(Mode::AbsX, Cpu::rol_val),
            0x66 => self.rmw(Mode::Zp, Cpu::ror_val),
            0x76 => self.rmw(Mode::ZpX, Cpu::ror_val),
            0x6E => self.rmw(Mode::Abs, Cpu::ror_val),
            0x7E => self.rmw(Mode::AbsX, Cpu::ror_val),
            // Jumps
            0x4C => self.pc = self.fetch16(),
            0x6C => {
                let ptr = self.fetch16();
                // 6502 bug: indirect JMP wraps within the page
                let lo = self.rd(ptr) as u16;
                let hi_addr = if ptr & 0x00FF == 0x00FF {
                    ptr & 0xFF00
                } else {
                    ptr + 1
                };
                let hi = self.rd(hi_addr) as u16;
                self.pc = (hi << 8) | lo;
            }
            0x20 => {
                // JSR: target high byte is fetched *after* the pushes.
                let lo = self.fetch8() as u16;
                self.rd(0x0100 + self.sp as u16); // dummy stack read
                self.push((self.pc >> 8) as u8);
                self.push(self.pc as u8);
                let hi = self.rd(self.pc) as u16;
                self.pc = (hi << 8) | lo;
            }
            0x60 => {
                // RTS
                self.implied_dummy();
                self.rd(0x0100 + self.sp as u16); // dummy stack read
                let lo = self.pop() as u16;
                let hi = self.pop() as u16;
                self.pc = (hi << 8) | lo;
                self.rd(self.pc); // dummy read at return address
                self.pc = self.pc.wrapping_add(1);
            }
            0x40 => {
                // RTI
                self.implied_dummy();
                self.rd(0x0100 + self.sp as u16);
                let v = self.pop();
                self.p = (v & !B) | U;
                let lo = self.pop() as u16;
                let hi = self.pop() as u16;
                self.pc = (hi << 8) | lo;
            }
            0x00 => {
                // BRK (vector can be hijacked by NMI)
                let _ = self.fetch8();
                self.push((self.pc >> 8) as u8);
                self.push(self.pc as u8);
                let vector = if self.nmi_pending {
                    self.nmi_pending = false;
                    0xFFFA
                } else {
                    0xFFFE
                };
                self.push(self.p | B | U);
                self.p |= I;
                let lo = self.rd(vector) as u16;
                let hi = self.rd(vector + 1) as u16;
                self.pc = (hi << 8) | lo;
                self.suppress_poll = true;
            }
            // Branches
            0x10 => self.branch(self.p & N == 0),
            0x30 => self.branch(self.p & N != 0),
            0x50 => self.branch(self.p & V == 0),
            0x70 => self.branch(self.p & V != 0),
            0x90 => self.branch(self.p & C == 0),
            0xB0 => self.branch(self.p & C != 0),
            0xD0 => self.branch(self.p & Z == 0),
            0xF0 => self.branch(self.p & Z != 0),
            // Flag ops
            0x18 => {
                self.implied_dummy();
                self.p &= !C;
            }
            0x38 => {
                self.implied_dummy();
                self.p |= C;
            }
            0x58 => {
                self.implied_dummy();
                self.p &= !I;
            }
            0x78 => {
                self.implied_dummy();
                self.p |= I;
            }
            0xD8 => {
                self.implied_dummy();
                self.p &= !D;
            }
            0xF8 => {
                self.implied_dummy();
                self.p |= D;
            }
            0xB8 => {
                self.implied_dummy();
                self.p &= !V;
            }
            // NOPs (official + unofficial)
            0xEA | 0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => self.implied_dummy(),
            0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => {
                let _ = self.fetch8();
            }
            0x04 | 0x44 | 0x64 => self.nop_read(Mode::Zp, true),
            0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => self.nop_read(Mode::ZpX, true),
            0x0C => self.nop_read(Mode::Abs, true),
            0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => self.nop_read(Mode::AbsX, true),
            // LAX
            0xA7 => self.lax(Mode::Zp),
            0xB7 => self.lax(Mode::ZpY),
            0xAF => self.lax(Mode::Abs),
            0xBF => self.lax(Mode::AbsY),
            0xA3 => self.lax(Mode::IndX),
            0xB3 => self.lax(Mode::IndY),
            // SAX
            0x87 => self.sax(Mode::Zp),
            0x97 => self.sax(Mode::ZpY),
            0x8F => self.sax(Mode::Abs),
            0x83 => self.sax(Mode::IndX),
            // DCP (DEC + CMP)
            0xC7 => self.dcp(Mode::Zp),
            0xD7 => self.dcp(Mode::ZpX),
            0xCF => self.dcp(Mode::Abs),
            0xDF => self.dcp(Mode::AbsX),
            0xDB => self.dcp(Mode::AbsY),
            0xC3 => self.dcp(Mode::IndX),
            0xD3 => self.dcp(Mode::IndY),
            // ISC (INC + SBC)
            0xE7 => self.isc(Mode::Zp),
            0xF7 => self.isc(Mode::ZpX),
            0xEF => self.isc(Mode::Abs),
            0xFF => self.isc(Mode::AbsX),
            0xFB => self.isc(Mode::AbsY),
            0xE3 => self.isc(Mode::IndX),
            0xF3 => self.isc(Mode::IndY),
            // SLO (ASL + ORA)
            0x07 => self.slo(Mode::Zp),
            0x17 => self.slo(Mode::ZpX),
            0x0F => self.slo(Mode::Abs),
            0x1F => self.slo(Mode::AbsX),
            0x1B => self.slo(Mode::AbsY),
            0x03 => self.slo(Mode::IndX),
            0x13 => self.slo(Mode::IndY),
            // RLA (ROL + AND)
            0x27 => self.rla(Mode::Zp),
            0x37 => self.rla(Mode::ZpX),
            0x2F => self.rla(Mode::Abs),
            0x3F => self.rla(Mode::AbsX),
            0x3B => self.rla(Mode::AbsY),
            0x23 => self.rla(Mode::IndX),
            0x33 => self.rla(Mode::IndY),
            // SRE (LSR + EOR)
            0x47 => self.sre(Mode::Zp),
            0x57 => self.sre(Mode::ZpX),
            0x4F => self.sre(Mode::Abs),
            0x5F => self.sre(Mode::AbsX),
            0x5B => self.sre(Mode::AbsY),
            0x43 => self.sre(Mode::IndX),
            0x53 => self.sre(Mode::IndY),
            // RRA (ROR + ADC)
            0x67 => self.rra(Mode::Zp),
            0x77 => self.rra(Mode::ZpX),
            0x6F => self.rra(Mode::Abs),
            0x7F => self.rra(Mode::AbsX),
            0x7B => self.rra(Mode::AbsY),
            0x63 => self.rra(Mode::IndX),
            0x73 => self.rra(Mode::IndY),
            // ANC: AND imm, then C = bit 7
            0x0B | 0x2B => {
                let v = self.fetch8();
                self.a &= v;
                self.set_zn(self.a);
                self.set_flag(C, self.a & 0x80 != 0);
            }
            // ALR: AND imm, then LSR A
            0x4B => {
                let v = self.fetch8();
                self.a &= v;
                let r = self.lsr_val(self.a);
                self.a = r;
            }
            // ARR: AND imm, ROR A; C = bit 6, V = bit 6 ^ bit 5
            0x6B => {
                let v = self.fetch8();
                self.a &= v;
                self.a = (self.a >> 1) | ((self.p & C) << 7);
                self.set_zn(self.a);
                self.set_flag(C, self.a & 0x40 != 0);
                self.set_flag(V, ((self.a >> 6) ^ (self.a >> 5)) & 1 != 0);
            }
            // AXS (SBX): X = (A & X) - imm, C set like CMP
            0xCB => {
                let v = self.fetch8();
                let t = self.a & self.x;
                self.set_flag(C, t >= v);
                self.x = t.wrapping_sub(v);
                self.set_zn(self.x);
            }
            // XAA (ANE), unstable: A = (A | magic) & X & imm
            0x8B => {
                let v = self.fetch8();
                self.a = (self.a | 0xEE) & self.x & v;
                self.set_zn(self.a);
            }
            // LXA (LAX imm), unstable: A = X = (A | magic) & imm
            0xAB => {
                let v = self.fetch8();
                let r = (self.a | 0xEE) & v;
                self.a = r;
                self.x = r;
                self.set_zn(r);
            }
            // SHA / SHX / SHY / TAS
            0x9F => {
                let base = self.fetch16();
                self.sh_write(base, self.y, self.a & self.x);
            }
            0x93 => {
                let zp = self.fetch8();
                let lo = self.rd(zp as u16) as u16;
                let hi = self.rd(zp.wrapping_add(1) as u16) as u16;
                let base = (hi << 8) | lo;
                self.sh_write(base, self.y, self.a & self.x);
            }
            0x9E => {
                let base = self.fetch16();
                self.sh_write(base, self.y, self.x);
            }
            0x9C => {
                let base = self.fetch16();
                self.sh_write(base, self.x, self.y);
            }
            0x9B => {
                let base = self.fetch16();
                self.sp = self.a & self.x;
                self.sh_write(base, self.y, self.sp);
            }
            // LAS: A = X = SP = mem & SP
            0xBB => {
                let a = self.addr(Mode::AbsY, true);
                let v = self.rd(a) & self.sp;
                self.a = v;
                self.x = v;
                self.sp = v;
                self.set_zn(v);
            }
            // KIL / JAM: halt the CPU by looping on the same opcode
            0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
                self.pc = self.pc.wrapping_sub(1);
            }
        }
    }
}
