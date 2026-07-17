use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// RAMBO-1 (mapper 64, Tengen 800032): an MMC3-like board with 8KB PRG and
/// 1KB CHR banking. It differs from the MMC3 in several ways:
///
/// * A third switchable PRG window: $8000/$A000/$C000 are all banked (R6, R7,
///   RF) while only $E000 is fixed to the last bank.
/// * An optional "full 1KB CHR" mode (`K`, $8000 bit 5) that splits the two
///   2KB CHR slots into four 1KB slots using extra registers R8/R9.
/// * Two selectable IRQ counter modes: the familiar scanline counter clocked
///   by PPU A12 rises, and a CPU-cycle counter clocked (through a /4
///   prescaler) every CPU cycle. The mode is latched from bit 0 of the value
///   written to $C001.
#[derive(Serialize, Deserialize)]
pub struct Rambo1 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    mirroring: Mirroring,
    // $8000 even: bits 0-3 command, bit 5 K (1KB CHR), bit 6 PRG mode,
    // bit 7 CHR A12 inversion.
    bank_select: u8,
    // R0..R9 plus RF. RF lives in index 15; 10-14 are unused.
    bank_regs: [u8; 16],
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_line: bool,
    // IRQ mode latched from $C001 bit 0: false = scanline (A12), true = CPU
    // cycle. In cycle mode the counter is clocked every 4 CPU cycles.
    irq_cpu_mode: bool,
    irq_prescaler: u8,
    last_a12: bool,
}

impl Rambo1 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Rambo1 {
            prg,
            chr,
            chr_is_ram,
            prg_ram: vec![0; 0x2000],
            mirroring,
            bank_select: 0,
            bank_regs: [0; 16],
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_line: false,
            irq_cpu_mode: false,
            irq_prescaler: 0,
            last_a12: false,
        }
    }

    /// Map a CPU address ($8000-$FFFF) to a PRG ROM offset (8KB banks).
    ///
    /// Unlike the MMC3 (which fixes two of its four windows), RAMBO-1 banks
    /// the first three windows and fixes only $E000 to the last bank:
    ///
    /// | window | P=0 | P=1 |
    /// |--------|-----|-----|
    /// | $8000  | R6  | RF  |
    /// | $A000  | R7  | R7  |
    /// | $C000  | RF  | R6  |
    /// | $E000  | last (fixed) |
    fn prg_offset(&self, addr: u16) -> usize {
        let banks = self.prg.len() / 0x2000;
        let last = banks - 1;
        let p = self.bank_select & 0x40 != 0;
        let r6 = self.bank_regs[6] as usize % banks;
        let r7 = self.bank_regs[7] as usize % banks;
        let rf = self.bank_regs[15] as usize % banks;
        let bank = match (addr >> 13) & 3 {
            0 => {
                if p {
                    rf
                } else {
                    r6
                }
            }
            1 => r7,
            2 => {
                if p {
                    r6
                } else {
                    rf
                }
            }
            _ => last,
        };
        bank * 0x2000 + (addr as usize & 0x1FFF)
    }

    /// Map a PPU address ($0000-$1FFF) to a CHR offset (1KB banks).
    ///
    /// With `K`=0 the layout matches the MMC3: R0/R1 select 2KB blocks and
    /// R2-R5 select 1KB blocks. With `K`=1 the two 2KB slots are split into
    /// four 1KB slots driven by R0,R8 (low half) and R1,R9 (second half);
    /// R2-R5 keep their 1KB roles. Bit 7 (`C`) inverts A12, swapping the
    /// 2KB region with the 1KB region (here, $0000-$0FFF vs $1000-$1FFF).
    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x400;
        let k = self.bank_select & 0x20 != 0;
        // Bit 7 swaps the "2KB" and "1KB" halves of pattern space.
        let a = if self.bank_select & 0x80 != 0 {
            addr ^ 0x1000
        } else {
            addr
        };
        let slot = a >> 10; // 0..=7, 1KB granularity
        let bank = if k {
            // Full 1KB mode: every slot is its own register.
            match slot {
                0 => self.bank_regs[0] as usize,
                1 => self.bank_regs[8] as usize,
                2 => self.bank_regs[1] as usize,
                3 => self.bank_regs[9] as usize,
                s => self.bank_regs[s as usize - 2] as usize,
            }
        } else {
            // 2KB mode (MMC3-like): R0/R1 are 2KB pairs, low bit ignored.
            match slot {
                0 => self.bank_regs[0] as usize & !1,
                1 => self.bank_regs[0] as usize | 1,
                2 => self.bank_regs[1] as usize & !1,
                3 => self.bank_regs[1] as usize | 1,
                s => self.bank_regs[s as usize - 2] as usize,
            }
        } % banks;
        bank * 0x400 + (addr as usize & 0x3FF)
    }

    /// Advance the shared 8-bit IRQ counter by one tick, applying RAMBO-1's
    /// reload-on-zero behavior and raising the line when it lands on zero.
    fn clock_irq(&mut self) {
        if self.irq_counter == 0 || self.irq_reload {
            // RAMBO-1 reloads with the latch ORed with the reload flag, giving
            // the documented off-by-one: a fresh reload counts one extra tick
            // versus the MMC3.
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 && self.irq_enabled {
            self.irq_line = true;
        }
    }

    /// Clock the scanline IRQ counter on each A12 rising edge seen on the PPU
    /// bus (only active while in scanline mode).
    fn watch_a12(&mut self, addr: u16) {
        let a12 = addr & 0x1000 != 0;
        if a12 && !self.last_a12 && !self.irq_cpu_mode {
            self.clock_irq();
        }
        self.last_a12 = a12;
    }
}

impl Mapper for Rambo1 {
    crate::impl_mapper_savestate!(chr, prg_ram);

    fn set_ram_sizes(&mut self, prg_ram: usize, chr_ram: usize) {
        if prg_ram > 0 {
            self.prg_ram = vec![0; prg_ram];
        }
        if chr_ram > 0 && self.chr_is_ram {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x6000..=0x7FFF => {
                self.prg_ram[(addr & 0x1FFF) as usize] = val;
            }
            0x8000..=0x9FFF => {
                if addr & 1 == 0 {
                    self.bank_select = val;
                } else {
                    // Commands 0-9 and 15 are valid; the upper bits of the
                    // command field index past R9 into the unused 10-14 slots,
                    // which is harmless (mask to 0xF keeps RF reachable).
                    let cmd = (self.bank_select & 0x0F) as usize;
                    self.bank_regs[cmd] = val;
                }
            }
            0xA000..=0xBFFF => {
                if addr & 1 == 0 {
                    // A four-screen board ignores the mirroring register.
                    if self.mirroring != Mirroring::FourScreen {
                        self.mirroring = if val & 1 != 0 {
                            Mirroring::Horizontal
                        } else {
                            Mirroring::Vertical
                        };
                    }
                }
                // $A001 (odd) is unused on RAMBO-1.
            }
            0xC000..=0xDFFF => {
                if addr & 1 == 0 {
                    // $C000: IRQ latch / reload value.
                    self.irq_latch = val;
                } else {
                    // $C001: bit 0 selects the IRQ mode and writing requests a
                    // counter reload (and resets the cycle prescaler).
                    self.irq_cpu_mode = val & 1 != 0;
                    self.irq_counter = 0;
                    self.irq_reload = true;
                    self.irq_prescaler = 0;
                }
            }
            0xE000..=0xFFFF => {
                if addr & 1 == 0 {
                    // $E000: disable + acknowledge.
                    self.irq_enabled = false;
                    self.irq_line = false;
                } else {
                    // $E001: enable (does not acknowledge a pending IRQ).
                    self.irq_enabled = true;
                }
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.watch_a12(addr);
        self.chr[self.chr_offset(addr)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        self.watch_a12(addr);
        if self.chr_is_ram {
            let off = self.chr_offset(addr);
            self.chr[off] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        Some(self.prg_ram[(addr & 0x1FFF) as usize])
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        Some(&self.prg_ram)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.prg_ram)
    }

    fn irq(&self) -> bool {
        self.irq_line
    }

    fn cpu_clock(&mut self) {
        // The CPU-cycle counter runs through a /4 prescaler.
        if self.irq_cpu_mode {
            self.irq_prescaler = self.irq_prescaler.wrapping_add(1);
            if self.irq_prescaler >= 4 {
                self.irq_prescaler = 0;
                self.clock_irq();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rambo() -> Rambo1 {
        // 8 PRG banks (64KB), 16 CHR banks (16KB); each byte = its bank index.
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Rambo1::new(prg, chr, Mirroring::Horizontal)
    }

    /// Helper: select command `cmd` then write `val` to that register.
    fn set_reg(m: &mut Rambo1, cmd: u8, val: u8) {
        m.cpu_write(0x8000, cmd); // keep mode/K/inv bits clear
        m.cpu_write(0x8001, val);
    }

    #[test]
    fn prg_layout_mode0() {
        let mut m = rambo();
        set_reg(&mut m, 6, 1); // R6
        set_reg(&mut m, 7, 2); // R7
        set_reg(&mut m, 15, 3); // RF
        // P=0: $8000=R6, $A000=R7, $C000=RF, $E000=last.
        assert_eq!(m.cpu_read(0x8000), 1);
        assert_eq!(m.cpu_read(0xA000), 2);
        assert_eq!(m.cpu_read(0xC000), 3);
        assert_eq!(m.cpu_read(0xE000), 7); // last of 8 banks
    }

    #[test]
    fn prg_layout_mode1() {
        let mut m = rambo();
        set_reg(&mut m, 6, 1);
        set_reg(&mut m, 7, 2);
        set_reg(&mut m, 15, 3);
        // Re-select with PRG mode bit (bit 6) set, without disturbing regs.
        m.cpu_write(0x8000, 0x40 | 15);
        m.cpu_write(0x8001, 3);
        // P=1: $8000=RF, $A000=R7, $C000=R6, $E000=last.
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xA000), 2);
        assert_eq!(m.cpu_read(0xC000), 1);
        assert_eq!(m.cpu_read(0xE000), 7);
    }

    #[test]
    fn chr_k0_banking() {
        let mut m = rambo();
        // K=0: R0/R1 are 2KB (low bit ignored), R2-R5 are 1KB.
        set_reg(&mut m, 0, 4); // 2KB pair -> 1KB banks 4,5 at $0000/$0400
        set_reg(&mut m, 1, 8); // 2KB pair -> 8,9 at $0800/$0C00
        set_reg(&mut m, 2, 12); // 1KB at $1000
        set_reg(&mut m, 3, 13); // 1KB at $1400
        set_reg(&mut m, 4, 14); // 1KB at $1800
        set_reg(&mut m, 5, 15); // 1KB at $1C00
        assert_eq!(m.ppu_read(0x0000), 4);
        assert_eq!(m.ppu_read(0x0400), 5);
        assert_eq!(m.ppu_read(0x0800), 8);
        assert_eq!(m.ppu_read(0x0C00), 9);
        assert_eq!(m.ppu_read(0x1000), 12);
        assert_eq!(m.ppu_read(0x1C00), 15);
    }

    #[test]
    fn chr_k0_a12_inversion() {
        let mut m = rambo();
        set_reg(&mut m, 0, 4); // 2KB pair
        set_reg(&mut m, 2, 12); // 1KB
        // Sanity before inversion.
        assert_eq!(m.ppu_read(0x0000), 4);
        assert_eq!(m.ppu_read(0x1000), 12);
        // Set C (bit 7) -> swaps the 2KB and 1KB halves.
        m.cpu_write(0x8000, 0x80);
        assert_eq!(m.ppu_read(0x1000), 4); // 2KB pair now high
        assert_eq!(m.ppu_read(0x0000), 12); // 1KB now low
    }

    #[test]
    fn chr_k1_banking() {
        let mut m = rambo();
        // Program registers first (with command bits only).
        set_reg(&mut m, 0, 4); // -> $0000 in K=1
        set_reg(&mut m, 8, 5); // -> $0400 in K=1
        set_reg(&mut m, 1, 6); // -> $0800 in K=1
        set_reg(&mut m, 9, 7); // -> $0C00 in K=1
        set_reg(&mut m, 2, 10); // -> $1000
        set_reg(&mut m, 3, 11); // -> $1400
        set_reg(&mut m, 4, 12); // -> $1800
        set_reg(&mut m, 5, 13); // -> $1C00
        // Enable K (bit 5) without changing the selected command/regs.
        m.cpu_write(0x8000, 0x20);
        assert_eq!(m.ppu_read(0x0000), 4);
        assert_eq!(m.ppu_read(0x0400), 5);
        assert_eq!(m.ppu_read(0x0800), 6);
        assert_eq!(m.ppu_read(0x0C00), 7);
        assert_eq!(m.ppu_read(0x1000), 10);
        assert_eq!(m.ppu_read(0x1400), 11);
        assert_eq!(m.ppu_read(0x1800), 12);
        assert_eq!(m.ppu_read(0x1C00), 13);
    }

    #[test]
    fn mirroring_control() {
        let mut m = rambo();
        m.cpu_write(0xA000, 0);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0xA000, 1);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn four_screen_ignores_mirroring_register() {
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        let mut m = Rambo1::new(prg, chr, Mirroring::FourScreen);
        m.cpu_write(0xA000, 0);
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
        m.cpu_write(0xA000, 1);
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
    }

    #[test]
    fn scanline_irq_counts_a12_rises() {
        let mut m = rambo();
        m.cpu_write(0xC000, 3); // latch = 3
        m.cpu_write(0xC001, 0); // scanline mode (bit0=0) + reload
        m.cpu_write(0xE001, 0); // enable
        // First rise reloads to 3, subsequent rises decrement: 3 -> 2 -> 1.
        for i in 0..3 {
            m.ppu_read(0x0000); // A12 low
            m.ppu_read(0x1000); // A12 rise
            assert!(!m.irq(), "IRQ too early at clock {i}");
        }
        m.ppu_read(0x0000);
        m.ppu_read(0x1000); // counter reaches 0
        assert!(m.irq());
        // $E000 acknowledges and disables.
        m.cpu_write(0xE000, 0);
        assert!(!m.irq());
    }

    #[test]
    fn scanline_irq_ignored_in_cpu_mode() {
        let mut m = rambo();
        m.cpu_write(0xC000, 1);
        m.cpu_write(0xC001, 1); // CPU-cycle mode + reload
        m.cpu_write(0xE001, 0); // enable
        // A12 rises must not clock the counter while in cycle mode.
        for _ in 0..16 {
            m.ppu_read(0x0000);
            m.ppu_read(0x1000);
        }
        assert!(!m.irq());
    }

    #[test]
    fn cpu_cycle_irq_counts_with_prescaler() {
        let mut m = rambo();
        m.cpu_write(0xC000, 2); // latch = 2
        m.cpu_write(0xC001, 1); // CPU-cycle mode (bit0=1) + reload
        m.cpu_write(0xE001, 0); // enable
        // /4 prescaler: 4 CPU cycles == one counter tick.
        // Tick 1 reloads to 2, tick 2 -> 1, tick 3 -> 0 (fires).
        for _ in 0..(2 * 4) {
            m.cpu_clock();
        }
        assert!(!m.irq(), "should not fire before reaching zero");
        for _ in 0..4 {
            m.cpu_clock();
        }
        assert!(m.irq(), "IRQ should fire when cycle counter hits zero");
        m.cpu_write(0xE000, 0); // ack
        assert!(!m.irq());
    }

    #[test]
    fn prg_ram_read_write() {
        let mut m = rambo();
        m.cpu_write(0x6123, 0xAB);
        assert_eq!(m.prg_ram_read(0x6123), Some(0xAB));
    }

    #[test]
    fn savestate_round_trip() {
        let mut m = rambo();
        set_reg(&mut m, 6, 5);
        m.cpu_write(0xC000, 9);
        m.cpu_write(0x6000, 0x77);
        let snap = m.save_state();
        let mut m2 = rambo();
        m2.load_state(&snap).unwrap();
        assert_eq!(m2.cpu_read(0x8000), 5);
        assert_eq!(m2.prg_ram_read(0x6000), Some(0x77));
    }
}
