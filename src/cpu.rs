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

#[rustfmt::skip]
const CYCLES: [u8; 256] = [
    7, 6, 2, 8, 3, 3, 5, 5, 3, 2, 2, 2, 4, 4, 6, 6,
    2, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,
    6, 6, 2, 8, 3, 3, 5, 5, 4, 2, 2, 2, 4, 4, 6, 6,
    2, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,
    6, 6, 2, 8, 3, 3, 5, 5, 3, 2, 2, 2, 3, 4, 6, 6,
    2, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,
    6, 6, 2, 8, 3, 3, 5, 5, 4, 2, 2, 2, 5, 4, 6, 6,
    2, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,
    2, 6, 2, 6, 3, 3, 3, 3, 2, 2, 2, 2, 4, 4, 4, 4,
    2, 6, 2, 6, 4, 4, 4, 4, 2, 5, 2, 5, 5, 5, 5, 5,
    2, 6, 2, 6, 3, 3, 3, 3, 2, 2, 2, 2, 4, 4, 4, 4,
    2, 5, 2, 5, 4, 4, 4, 4, 2, 4, 2, 4, 4, 4, 4, 4,
    2, 6, 2, 8, 3, 3, 5, 5, 2, 2, 2, 2, 4, 4, 6, 6,
    2, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,
    2, 6, 2, 8, 3, 3, 5, 5, 2, 2, 2, 2, 4, 4, 6, 6,
    2, 5, 2, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,
];

pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub p: u8,
    pub cycles: u64,
    extra: u8, // page-cross / branch penalty cycles for current instruction
    pub bus: Bus,
}

impl Cpu {
    pub fn new(bus: Bus) -> Self {
        Cpu { a: 0, x: 0, y: 0, sp: 0xFD, pc: 0, p: I | U, cycles: 0, extra: 0, bus }
    }

    pub fn reset(&mut self) {
        self.pc = self.read16(0xFFFC);
        self.sp = 0xFD;
        self.p = I | U;
        self.cycles = 7;
        self.bus.tick(7);
    }

    pub fn step(&mut self) -> u64 {
        if self.bus.take_nmi() {
            self.interrupt(0xFFFA);
            self.cycles += 7;
            self.bus.tick(7);
            self.post_tick();
            return 7;
        }
        let op = self.fetch8();
        self.extra = 0;
        self.exec(op);
        let total = CYCLES[op as usize] as u64 + self.extra as u64;
        self.cycles += total;
        self.bus.tick(total);
        self.post_tick();
        total
    }

    // OAM DMA stall: tick the PPU through the stalled cycles
    fn post_tick(&mut self) {
        let stall = self.bus.take_dma_stall();
        if stall > 0 {
            self.cycles += stall;
            self.bus.tick(stall);
        }
    }

    fn interrupt(&mut self, vector: u16) {
        self.push16(self.pc);
        self.push((self.p & !B) | U);
        self.p |= I;
        self.pc = self.read16(vector);
    }

    // ---- memory helpers ----

    fn fetch8(&mut self) -> u8 {
        let v = self.bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    fn fetch16(&mut self) -> u16 {
        let lo = self.fetch8() as u16;
        let hi = self.fetch8() as u16;
        (hi << 8) | lo
    }

    fn read16(&mut self, addr: u16) -> u16 {
        let lo = self.bus.read(addr) as u16;
        let hi = self.bus.read(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    fn push(&mut self, v: u8) {
        self.bus.write(0x0100 + self.sp as u16, v);
        self.sp = self.sp.wrapping_sub(1);
    }

    fn push16(&mut self, v: u16) {
        self.push((v >> 8) as u8);
        self.push(v as u8);
    }

    fn pop(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.bus.read(0x0100 + self.sp as u16)
    }

    fn pop16(&mut self) -> u16 {
        let lo = self.pop() as u16;
        let hi = self.pop() as u16;
        (hi << 8) | lo
    }

    // ---- addressing ----

    fn addr(&mut self, m: Mode, penalty: bool) -> u16 {
        match m {
            Mode::Imm => {
                let a = self.pc;
                self.pc = self.pc.wrapping_add(1);
                a
            }
            Mode::Zp => self.fetch8() as u16,
            Mode::ZpX => self.fetch8().wrapping_add(self.x) as u16,
            Mode::ZpY => self.fetch8().wrapping_add(self.y) as u16,
            Mode::Abs => self.fetch16(),
            Mode::AbsX => {
                let base = self.fetch16();
                let a = base.wrapping_add(self.x as u16);
                if penalty && (base & 0xFF00) != (a & 0xFF00) {
                    self.extra += 1;
                }
                a
            }
            Mode::AbsY => {
                let base = self.fetch16();
                let a = base.wrapping_add(self.y as u16);
                if penalty && (base & 0xFF00) != (a & 0xFF00) {
                    self.extra += 1;
                }
                a
            }
            Mode::IndX => {
                let zp = self.fetch8().wrapping_add(self.x);
                let lo = self.bus.read(zp as u16) as u16;
                let hi = self.bus.read(zp.wrapping_add(1) as u16) as u16;
                (hi << 8) | lo
            }
            Mode::IndY => {
                let zp = self.fetch8();
                let lo = self.bus.read(zp as u16) as u16;
                let hi = self.bus.read(zp.wrapping_add(1) as u16) as u16;
                let base = (hi << 8) | lo;
                let a = base.wrapping_add(self.y as u16);
                if penalty && (base & 0xFF00) != (a & 0xFF00) {
                    self.extra += 1;
                }
                a
            }
        }
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

    fn lda(&mut self, m: Mode) {
        let a = self.addr(m, true);
        self.a = self.bus.read(a);
        self.set_zn(self.a);
    }

    fn ldx(&mut self, m: Mode) {
        let a = self.addr(m, true);
        self.x = self.bus.read(a);
        self.set_zn(self.x);
    }

    fn ldy(&mut self, m: Mode) {
        let a = self.addr(m, true);
        self.y = self.bus.read(a);
        self.set_zn(self.y);
    }

    fn sta(&mut self, m: Mode) {
        let a = self.addr(m, false);
        self.bus.write(a, self.a);
    }

    fn stx(&mut self, m: Mode) {
        let a = self.addr(m, false);
        self.bus.write(a, self.x);
    }

    fn sty(&mut self, m: Mode) {
        let a = self.addr(m, false);
        self.bus.write(a, self.y);
    }

    fn and(&mut self, m: Mode) {
        let a = self.addr(m, true);
        self.a &= self.bus.read(a);
        self.set_zn(self.a);
    }

    fn ora(&mut self, m: Mode) {
        let a = self.addr(m, true);
        self.a |= self.bus.read(a);
        self.set_zn(self.a);
    }

    fn eor(&mut self, m: Mode) {
        let a = self.addr(m, true);
        self.a ^= self.bus.read(a);
        self.set_zn(self.a);
    }

    fn bit(&mut self, m: Mode) {
        let a = self.addr(m, false);
        let v = self.bus.read(a);
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
        let a = self.addr(m, true);
        let v = self.bus.read(a);
        self.adc_val(v);
    }

    fn sbc(&mut self, m: Mode) {
        let a = self.addr(m, true);
        let v = self.bus.read(a);
        self.adc_val(v ^ 0xFF);
    }

    fn compare(&mut self, m: Mode, reg: u8) {
        let a = self.addr(m, true);
        let v = self.bus.read(a);
        self.set_flag(C, reg >= v);
        self.set_zn(reg.wrapping_sub(v));
    }

    fn inc(&mut self, m: Mode) {
        let a = self.addr(m, false);
        let v = self.bus.read(a).wrapping_add(1);
        self.bus.write(a, v);
        self.set_zn(v);
    }

    fn dec(&mut self, m: Mode) {
        let a = self.addr(m, false);
        let v = self.bus.read(a).wrapping_sub(1);
        self.bus.write(a, v);
        self.set_zn(v);
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

    fn rmw(&mut self, m: Mode, f: fn(&mut Cpu, u8) -> u8) {
        let a = self.addr(m, false);
        let v = self.bus.read(a);
        let r = f(self, v);
        self.bus.write(a, r);
    }

    fn branch(&mut self, cond: bool) {
        let off = self.fetch8() as i8;
        if cond {
            self.extra += 1;
            let old = self.pc;
            self.pc = old.wrapping_add(off as u16);
            if (old & 0xFF00) != (self.pc & 0xFF00) {
                self.extra += 1;
            }
        }
    }

    fn nop_read(&mut self, m: Mode, penalty: bool) {
        let a = self.addr(m, penalty);
        let _ = self.bus.read(a);
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
                self.x = self.a;
                self.set_zn(self.x);
            }
            0xA8 => {
                self.y = self.a;
                self.set_zn(self.y);
            }
            0x8A => {
                self.a = self.x;
                self.set_zn(self.a);
            }
            0x98 => {
                self.a = self.y;
                self.set_zn(self.a);
            }
            0xBA => {
                self.x = self.sp;
                self.set_zn(self.x);
            }
            0x9A => self.sp = self.x,
            // Stack
            0x48 => self.push(self.a),
            0x08 => self.push(self.p | B | U),
            0x68 => {
                self.a = self.pop();
                self.set_zn(self.a);
            }
            0x28 => {
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
            0xE6 => self.inc(Mode::Zp),
            0xF6 => self.inc(Mode::ZpX),
            0xEE => self.inc(Mode::Abs),
            0xFE => self.inc(Mode::AbsX),
            0xC6 => self.dec(Mode::Zp),
            0xD6 => self.dec(Mode::ZpX),
            0xCE => self.dec(Mode::Abs),
            0xDE => self.dec(Mode::AbsX),
            // INX/INY/DEX/DEY
            0xE8 => {
                self.x = self.x.wrapping_add(1);
                self.set_zn(self.x);
            }
            0xC8 => {
                self.y = self.y.wrapping_add(1);
                self.set_zn(self.y);
            }
            0xCA => {
                self.x = self.x.wrapping_sub(1);
                self.set_zn(self.x);
            }
            0x88 => {
                self.y = self.y.wrapping_sub(1);
                self.set_zn(self.y);
            }
            // Shifts: accumulator
            0x0A => {
                let v = self.a;
                self.a = self.asl_val(v);
            }
            0x4A => {
                let v = self.a;
                self.a = self.lsr_val(v);
            }
            0x2A => {
                let v = self.a;
                self.a = self.rol_val(v);
            }
            0x6A => {
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
                let lo = self.bus.read(ptr) as u16;
                let hi_addr = if ptr & 0x00FF == 0x00FF { ptr & 0xFF00 } else { ptr + 1 };
                let hi = self.bus.read(hi_addr) as u16;
                self.pc = (hi << 8) | lo;
            }
            0x20 => {
                let target = self.fetch16();
                let ret = self.pc.wrapping_sub(1);
                self.push16(ret);
                self.pc = target;
            }
            0x60 => {
                self.pc = self.pop16().wrapping_add(1);
            }
            0x40 => {
                let v = self.pop();
                self.p = (v & !B) | U;
                self.pc = self.pop16();
            }
            0x00 => {
                let _ = self.fetch8();
                self.push16(self.pc);
                self.push(self.p | B | U);
                self.p |= I;
                self.pc = self.read16(0xFFFE);
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
            0x18 => self.p &= !C,
            0x38 => self.p |= C,
            0x58 => self.p &= !I,
            0x78 => self.p |= I,
            0xD8 => self.p &= !D,
            0xF8 => self.p |= D,
            0xB8 => self.p &= !V,
            // NOPs (official + unofficial)
            0xEA | 0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => {}
            0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => {
                let _ = self.fetch8();
            }
            0x04 | 0x44 | 0x64 => self.nop_read(Mode::Zp, false),
            0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => self.nop_read(Mode::ZpX, false),
            0x0C => self.nop_read(Mode::Abs, false),
            0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => self.nop_read(Mode::AbsX, true),
            _ => panic!("unknown opcode {:02X} at {:04X}", op, self.pc.wrapping_sub(1)),
        }
    }
}
