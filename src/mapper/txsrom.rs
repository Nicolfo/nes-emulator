use super::{Mapper, Mirroring, NtTarget, mirror_nt};
use serde::{Deserialize, Serialize};

/// TxSROM (mapper 118): MMC3 with per-quadrant nametable selection. PRG/CHR
/// banking, scanline IRQ on A12, and PRG RAM protect are identical to MMC3.
/// The $A000 mirroring register does not switch H/V; instead nametable routing
/// is driven by bit 7 of the active CHR bank register for each 1KB quadrant.
#[derive(Serialize, Deserialize)]
pub struct Txsrom {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    mirroring: Mirroring,
    // $8000: bits 0-2 select which bank register $8001 writes; bit 6 PRG
    // mode; bit 7 CHR A12 inversion.
    bank_select: u8,
    bank_regs: [u8; 8],
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_line: bool,
    last_a12: bool,
    // $A001: bit 7 enables PRG RAM, bit 6 write-protects it. Power-on is
    // undefined on hardware; default to enabled + writable so games that
    // never touch $A001 keep working.
    ram_protect: u8,
}

impl Txsrom {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Txsrom {
            prg,
            chr,
            chr_is_ram,
            prg_ram: vec![0; 0x2000],
            mirroring,
            bank_select: 0,
            bank_regs: [0; 8],
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_line: false,
            last_a12: false,
            ram_protect: 0x80,
        }
    }

    /// Map a CPU address ($8000-$FFFF) to a PRG ROM offset (8KB banks).
    fn prg_offset(&self, addr: u16) -> usize {
        let banks = self.prg.len() / 0x2000;
        let last = banks - 1;
        let mode = self.bank_select & 0x40 != 0;
        let bank = match (addr >> 13) & 3 {
            0 => {
                if mode {
                    last - 1
                } else {
                    self.bank_regs[6] as usize % banks
                }
            }
            1 => self.bank_regs[7] as usize % banks,
            2 => {
                if mode {
                    self.bank_regs[6] as usize % banks
                } else {
                    last - 1
                }
            }
            _ => last,
        };
        bank * 0x2000 + (addr as usize & 0x1FFF)
    }

    /// Map a PPU address ($0000-$1FFF) to a CHR offset (1KB banks).
    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x400;
        // Bit 7 swaps the 2KB and 1KB halves of the pattern space.
        let a = if self.bank_select & 0x80 != 0 {
            addr ^ 0x1000
        } else {
            addr
        };
        let bank = match a >> 10 {
            0 => self.bank_regs[0] as usize & !1,
            1 => self.bank_regs[0] as usize | 1,
            2 => self.bank_regs[1] as usize & !1,
            3 => self.bank_regs[1] as usize | 1,
            k => self.bank_regs[k as usize - 2] as usize,
        } % banks;
        bank * 0x400 + (addr as usize & 0x3FF)
    }

    /// Clock the IRQ counter on each A12 rising edge seen on the PPU bus.
    fn watch_a12(&mut self, addr: u16) {
        let a12 = addr & 0x1000 != 0;
        if a12 && !self.last_a12 {
            if self.irq_counter == 0 || self.irq_reload {
                self.irq_counter = self.irq_latch;
                self.irq_reload = false;
            } else {
                self.irq_counter -= 1;
            }
            if self.irq_counter == 0 && self.irq_enabled {
                self.irq_line = true;
            }
        }
        self.last_a12 = a12;
    }
}

impl Mapper for Txsrom {
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
                if self.ram_protect & 0xC0 == 0x80 {
                    self.prg_ram[(addr & 0x1FFF) as usize] = val;
                }
            }
            0x8000..=0x9FFF => {
                if addr & 1 == 0 {
                    self.bank_select = val;
                } else {
                    self.bank_regs[(self.bank_select & 7) as usize] = val;
                }
            }
            0xA000..=0xBFFF => {
                // Unlike MMC3, the $A000 (even) mirroring register is ignored:
                // nametable routing comes from the CHR bank registers' bit 7.
                // The $A001 (odd) PRG RAM protect register is still honored.
                if addr & 1 != 0 {
                    self.ram_protect = val;
                }
            }
            0xC000..=0xDFFF => {
                if addr & 1 == 0 {
                    self.irq_latch = val;
                } else {
                    self.irq_counter = 0;
                    self.irq_reload = true;
                }
            }
            0xE000..=0xFFFF => {
                if addr & 1 == 0 {
                    self.irq_enabled = false;
                    self.irq_line = false;
                } else {
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
        // Stored header mirroring; unused for routing (see nt_target) but kept
        // for the FourScreen path.
        self.mirroring
    }

    fn nt_target(&mut self, addr: u16) -> NtTarget {
        // FourScreen boards (header pad) bypass mapper control.
        if self.mirroring == Mirroring::FourScreen {
            return NtTarget::Ciram(mirror_nt(Mirroring::FourScreen, addr));
        }
        let idx = ((addr >> 10) & 3) as usize; // which 1KB nametable quadrant
        // A nametable fetch ($2xxx) has PPU A12 = 0, so the CHR decode that
        // drives CIRAM A10 picks the half opposite to the pattern tables:
        // with inversion off A12=0 maps the two 2KB regs (R0/R1), and with
        // inversion on it maps the four 1KB regs (R2-R5).
        let reg = if self.bank_select & 0x80 != 0 {
            // inversion on: quadrants use the four 1KB regs R2,R3,R4,R5
            [
                self.bank_regs[2],
                self.bank_regs[3],
                self.bank_regs[4],
                self.bank_regs[5],
            ][idx]
        } else {
            // inversion off: quadrants use the two 2KB regs R0,R0,R1,R1
            [
                self.bank_regs[0],
                self.bank_regs[0],
                self.bank_regs[1],
                self.bank_regs[1],
            ][idx]
        };
        let a10 = ((reg >> 7) & 1) as u16;
        NtTarget::Ciram((a10 << 10) | (addr & 0x3FF))
    }

    fn prg_ram_read(&mut self, addr: u16) -> Option<u8> {
        if self.ram_protect & 0x80 == 0 {
            return None; // open bus
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txsrom() -> Txsrom {
        // 4 PRG banks (32KB), 8 CHR banks (8KB); each byte = its bank index.
        let prg: Vec<u8> = (0..4 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Txsrom::new(prg, chr, Mirroring::Horizontal)
    }

    #[test]
    fn prg_fixed_banks() {
        let mut m = txsrom();
        // Mode 0: $C000 = second-last, $E000 = last.
        assert_eq!(m.cpu_read(0xC000), 2);
        assert_eq!(m.cpu_read(0xE000), 3);
        // Mode 1: $8000 = second-last instead.
        m.cpu_write(0x8000, 0x40);
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0xE000), 3);
    }

    #[test]
    fn prg_switchable_banks() {
        let mut m = txsrom();
        m.cpu_write(0x8000, 6); // select R6
        m.cpu_write(0x8001, 1);
        m.cpu_write(0x8000, 7); // select R7
        m.cpu_write(0x8001, 0);
        assert_eq!(m.cpu_read(0x8000), 1);
        assert_eq!(m.cpu_read(0xA000), 0);
    }

    #[test]
    fn chr_banking_and_inversion() {
        let mut m = txsrom();
        m.cpu_write(0x8000, 0); // R0: 2KB at $0000 (low bit ignored)
        m.cpu_write(0x8001, 5);
        m.cpu_write(0x8000, 2); // R2: 1KB at $1000
        m.cpu_write(0x8001, 7);
        assert_eq!(m.ppu_read(0x0000), 4);
        assert_eq!(m.ppu_read(0x0400), 5);
        assert_eq!(m.ppu_read(0x1000), 7);
        // Bit 7 swaps the halves.
        m.cpu_write(0x8000, 0x80);
        assert_eq!(m.ppu_read(0x1000), 4);
        assert_eq!(m.ppu_read(0x0000), 7);
    }

    #[test]
    fn mirroring_register_is_ignored() {
        let mut m = txsrom();
        // Writes to $A000 must not change the stored header mirroring.
        m.cpu_write(0xA000, 0);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0xA000, 1);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn nt_target_uses_chr_reg_bit7_inversion_off() {
        let mut m = txsrom();
        // Inversion off (bank_select bit 7 = 0): quadrants use R0,R0,R1,R1.
        m.cpu_write(0x8000, 1); // select R1
        m.cpu_write(0x8001, 0x80); // R1 bit7 set
        // Quadrant 0/1 ($2000/$2400) use R0 (clear) -> low nametable.
        assert_eq!(m.nt_target(0x2000), NtTarget::Ciram(0x000));
        assert_eq!(m.nt_target(0x2400), NtTarget::Ciram(0x000));
        // Quadrant 2/3 ($2800/$2C00) use R1 (bit7 set) -> 0x400 set.
        match m.nt_target(0x2812) {
            NtTarget::Ciram(off) => assert_eq!(off, 0x412),
            other => panic!("expected Ciram, got {other:?}"),
        }
        match m.nt_target(0x2C00) {
            NtTarget::Ciram(off) => assert_eq!(off & 0x400, 0x400),
            other => panic!("expected Ciram, got {other:?}"),
        }
    }

    #[test]
    fn nt_target_uses_chr_reg_bit7_inversion_on() {
        let mut m = txsrom();
        // Inversion on (bank_select bit 7 = 1): quadrants use R2,R3,R4,R5.
        // Set bit 7 of R3 (quadrant 1 -> $2400) and leave the rest clear.
        m.cpu_write(0x8000, 0x80 | 3); // bit7 set + select R3
        m.cpu_write(0x8001, 0x80);
        // Quadrant 0 ($2000): R2 bit7 clear -> low nametable, no 0x400.
        assert_eq!(m.nt_target(0x2000), NtTarget::Ciram(0x000));
        // Quadrant 1 ($2400): R3 bit7 set -> 0x400 bit set.
        assert_eq!(m.nt_target(0x2412), NtTarget::Ciram(0x412));
        // Quadrant 2 ($2800): R4 bit7 clear.
        assert_eq!(m.nt_target(0x2800), NtTarget::Ciram(0x000));
    }

    #[test]
    fn four_screen_bypasses_mapper_routing() {
        let prg: Vec<u8> = (0..4 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x400).map(|i| (i / 0x400) as u8).collect();
        let mut m = Txsrom::new(prg, chr, Mirroring::FourScreen);
        // Even with a CHR reg bit7 set, four-screen routes linearly.
        m.cpu_write(0x8000, 3);
        m.cpu_write(0x8001, 0x80);
        let offs: Vec<NtTarget> = [0x2000, 0x2400, 0x2800, 0x2C00]
            .iter()
            .map(|&a| m.nt_target(a))
            .collect();
        assert_eq!(
            offs,
            vec![
                NtTarget::Ciram(0x000),
                NtTarget::Ciram(0x400),
                NtTarget::Ciram(0x800),
                NtTarget::Ciram(0xC00),
            ]
        );
    }

    #[test]
    fn irq_counts_a12_rises() {
        let mut m = txsrom();
        m.cpu_write(0xC000, 3); // latch
        m.cpu_write(0xC001, 0); // reload on next clock
        m.cpu_write(0xE001, 0); // enable
        // Each low->high A12 transition is one clock.
        for i in 0..3 {
            m.ppu_read(0x0000); // A12 low
            m.ppu_read(0x1000); // A12 rise: reload to 3, then 2, 1
            assert!(!m.irq(), "IRQ too early at clock {i}");
        }
        m.ppu_read(0x0000);
        m.ppu_read(0x1000); // counter hits 0
        assert!(m.irq());
        // $E000 acknowledges and disables.
        m.cpu_write(0xE000, 0);
        assert!(!m.irq());
    }

    #[test]
    fn prg_ram_protect() {
        let mut m = txsrom();
        m.cpu_write(0x6000, 0xAA);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAA));
        // Write-protected: writes ignored, reads still work.
        m.cpu_write(0xA001, 0xC0);
        m.cpu_write(0x6000, 0xBB);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xAA));
        // Disabled: open bus.
        m.cpu_write(0xA001, 0x00);
        assert_eq!(m.prg_ram_read(0x6000), None);
        // Re-enabled writable.
        m.cpu_write(0xA001, 0x80);
        m.cpu_write(0x6000, 0xBB);
        assert_eq!(m.prg_ram_read(0x6000), Some(0xBB));
    }

    #[test]
    fn prg_ram_read_write() {
        let mut m = txsrom();
        m.cpu_write(0x6123, 0xAB);
        assert_eq!(m.prg_ram_read(0x6123), Some(0xAB));
    }
}
