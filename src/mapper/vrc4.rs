use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Konami VRC2 and VRC4 family (iNES mappers 21, 22, 23, 25).
///
/// One chip family covers several boards that differ only in *which* CPU
/// address lines carry the two low register-select bits. The high nibble of
/// the address ($8/$9/$A.../$F) picks the register group; two further bits
/// pick one of four registers within the group. Those two bits are wired to
/// different address pins per board:
///
/// | iNES | board(s)        | low-bit lines            |
/// |------|-----------------|--------------------------|
/// | 21   | VRC4a / VRC4c   | A1,A2  and  A6,A7        |
/// | 22   | VRC2a           | A1,A0  (SWAPPED)         |
/// | 23   | VRC4f/VRC2b + VRC4e | A0,A1  and  A2,A3    |
/// | 25   | VRC4b + VRC4d   | A1,A0  and  A3,A2 (SWAPPED) |
///
/// Without NES 2.0 submapper bits we cannot tell *which* of a mapper number's
/// two pin assignments a particular cartridge uses. The standard
/// submapper-agnostic trick is to OR together both candidate line pairs: a
/// real game only ever drives one pair (the unused lines stay 0 for its
/// register writes), so ORing yields the correct 2-bit index either way.
/// Mapper 22 is the lone exception: its two low bits are physically swapped
/// (A1 -> bit 0, A0 -> bit 1), so we build its index explicitly.
///
/// VRC2 (mapper 22) has no IRQ and a CHR-banking quirk; VRC4 (21/23/25) is the
/// full superset. We always model the VRC4 superset for 21/23/25 since the
/// extra registers are harmless for VRC2 titles that happen to use those
/// numbers, and we set `vrc2 = (mapper == 22)` to gate the quirks.
#[derive(Clone, Serialize, Deserialize)]
pub struct Vrc4 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    #[serde(with = "crate::savestate::byte_array")]
    prg_ram: [u8; 0x2000],
    mirroring: Mirroring,
    /// iNES mapper number; selects the address-line scrambling.
    mapper: u8,
    /// True for mapper 22 (VRC2a): no IRQ, CHR-granularity quirk, A0/A1 swap.
    vrc2: bool,
    /// $8000: low PRG 8KB bank (swappable region A).
    prg_bank8: u8,
    /// $A000: PRG 8KB bank for $A000-$BFFF.
    prg_bank_a: u8,
    /// $9002 bit1 (VRC4 only): swaps whether $8000 or $C000 is swappable.
    prg_swap: bool,
    /// Eight 1KB CHR banks, each assembled from a low and a high nibble write.
    chr_banks: [u16; 8],
    irq: VrcIrq,
}

impl Vrc4 {
    /// Construct for any of the four iNES numbers. `mapper` must be 21, 22, 23
    /// or 25; other values behave like a generic VRC4 (no scrambling beyond
    /// A0/A1).
    pub fn new(mapper: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        // VRC2/VRC4 boards with CHR-RAM use 8KB.
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        Vrc4 {
            prg,
            chr,
            chr_is_ram,
            prg_ram: [0; 0x2000],
            mirroring,
            mapper,
            vrc2: mapper == 22,
            prg_bank8: 0,
            prg_bank_a: 0,
            prg_swap: false,
            chr_banks: [0; 8],
            irq: VrcIrq::new(),
        }
    }

    /// Collapse a raw $8000-$FFFF address into a canonical register:
    /// `(group, index)` where `group` is the high nibble shifted to 0..=7
    /// (so $8xxx->0, $9xxx->1, ... $Fxxx->7) and `index` is the 2-bit
    /// in-group selector after de-scrambling the board's address lines.
    fn reg(&self, addr: u16) -> (u16, u16) {
        let group = (addr >> 12) & 7; // $8->0 .. $F->7
        let index = match self.mapper {
            // VRC4a uses A1,A2; VRC4c uses A6,A7. OR both candidate pairs.
            21 => {
                let b0 = ((addr >> 1) & 1) | ((addr >> 6) & 1);
                let b1 = ((addr >> 2) & 1) | ((addr >> 7) & 1);
                b0 | (b1 << 1)
            }
            // VRC2a: A1 is low bit 0, A0 is low bit 1 (the two are SWAPPED).
            22 => ((addr >> 1) & 1) | ((addr & 1) << 1),
            // VRC4f/VRC2b uses A0,A1; VRC4e uses A2,A3. OR both candidate pairs.
            23 => {
                let b0 = (addr & 1) | ((addr >> 2) & 1);
                let b1 = ((addr >> 1) & 1) | ((addr >> 3) & 1);
                b0 | (b1 << 1)
            }
            // VRC4b uses A1,A0; VRC4d uses A3,A2 - both SWAPPED relative to
            // mapper 23 (the low selector bit comes from the higher address
            // line). OR both candidate pairs.
            25 => {
                let b0 = ((addr >> 1) & 1) | ((addr >> 3) & 1); // A1 or A3
                let b1 = (addr & 1) | ((addr >> 2) & 1); // A0 or A2
                b0 | (b1 << 1)
            }
            // Fallback: straight A0/A1.
            _ => addr & 3,
        };
        (group, index)
    }

    /// PRG ROM offset for a CPU read in $8000-$FFFF.
    fn prg_offset(&self, addr: u16) -> usize {
        let banks = self.prg.len() / 0x2000;
        let last = banks - 1;
        // The two fixed banks are always the last two 8KB banks.
        let bank = match (addr >> 13) & 3 {
            // $8000-$9FFF: swappable region A, or fixed second-to-last when
            // PRG-swap mode moves the swappable window to $C000.
            0 => {
                if self.prg_swap {
                    last - 1
                } else {
                    self.prg_bank8 as usize % banks
                }
            }
            // $A000-$BFFF: always the $A000 register's bank.
            1 => self.prg_bank_a as usize % banks,
            // $C000-$DFFF: swappable region B in swap mode, else fixed
            // second-to-last.
            2 => {
                if self.prg_swap {
                    self.prg_bank8 as usize % banks
                } else {
                    last - 1
                }
            }
            // $E000-$FFFF: always the last 8KB bank.
            _ => last,
        };
        bank * 0x2000 + (addr as usize & 0x1FFF)
    }

    /// CHR ROM/RAM offset for a PPU access in $0000-$1FFF.
    fn chr_offset(&self, addr: u16) -> usize {
        let banks = self.chr.len() / 0x400;
        let mut bank = self.chr_banks[(addr >> 10) as usize & 7] as usize;
        // VRC2 CHR quirk: the very earliest VRC2a (mapper 22) only wires the
        // upper bits of each CHR bank register through to the ROM, so the
        // programmed value selects banks in 2KB granularity -- effectively the
        // written value is right-shifted by one when addressing 1KB banks.
        // Isolate the quirk here so it is easy to flip if a submapper says
        // otherwise.
        if self.vrc2 {
            bank >>= 1;
        }
        bank %= banks;
        bank * 0x400 + (addr as usize & 0x3FF)
    }

    /// Apply a CHR-bank nibble write. `slot` is 0..=7, `high` selects the upper
    /// nibble (odd register index) vs the low nibble (even register index).
    fn write_chr_nibble(&mut self, slot: usize, high: bool, val: u8) {
        let cur = self.chr_banks[slot];
        self.chr_banks[slot] = if high {
            // VRC4 CHR bank registers are up to 9 bits wide (high write holds
            // bits 4..=8). VRC2 only needs the low byte but the masking here is
            // harmless for it.
            (cur & 0x0F) | (((val as u16) & 0x1F) << 4)
        } else {
            (cur & !0x0F) | (val as u16 & 0x0F)
        };
    }
}

impl Mapper for Vrc4 {
    crate::impl_mapper_savestate!(chr_is_ram = chr_is_ram);
    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        if addr < 0x8000 {
            if (0x6000..0x8000).contains(&addr) {
                self.prg_ram[(addr & 0x1FFF) as usize] = val;
            }
            return;
        }
        let (group, index) = self.reg(addr);
        match group {
            // $8000-$8FFF: low PRG 8KB bank.
            0 => self.prg_bank8 = val & 0x1F,
            // $9000-$9FFF: index 0/1 -> mirroring, index 2 -> control (PRG swap).
            1 => match index {
                0 | 1 => {
                    if self.mirroring != Mirroring::FourScreen {
                        self.mirroring = if self.vrc2 {
                            // VRC2 has only 1 mirroring bit.
                            if val & 1 != 0 {
                                Mirroring::Horizontal
                            } else {
                                Mirroring::Vertical
                            }
                        } else {
                            match val & 3 {
                                0 => Mirroring::Vertical,
                                1 => Mirroring::Horizontal,
                                2 => Mirroring::SingleScreenLo,
                                _ => Mirroring::SingleScreenHi,
                            }
                        };
                    }
                }
                // $9002 (VRC4): bit1 selects which PRG window is swappable.
                _ => {
                    if !self.vrc2 {
                        self.prg_swap = val & 0x02 != 0;
                    }
                }
            },
            // $A000-$AFFF: PRG 8KB bank for $A000-$BFFF.
            2 => self.prg_bank_a = val & 0x1F,
            // $B000-$EFFF: eight 1KB CHR banks, two nibbles each.
            // Group 3 ($B) -> slots 0,1; 4 ($C) -> 2,3; 5 ($D) -> 4,5;
            // 6 ($E) -> 6,7. Even index = low nibble, odd index = high nibble.
            3..=6 => {
                let base = ((group - 3) * 2) as usize; // 0,2,4,6
                let slot = base + (index >> 1) as usize; // index 0/1 vs 2/3
                let high = index & 1 != 0;
                self.write_chr_nibble(slot, high, val);
            }
            // $F000-$FFFF: VRC4 IRQ (ignored on VRC2).
            _ => {
                if !self.vrc2 {
                    match index {
                        0 => self.irq.write_latch_lo(val),
                        1 => self.irq.write_latch_hi(val),
                        2 => self.irq.write_control(val),
                        _ => self.irq.ack(),
                    }
                }
            }
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.chr[self.chr_offset(addr)]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
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
        self.irq.line
    }

    fn cpu_clock(&mut self) {
        if !self.vrc2 {
            self.irq.clock();
        }
    }
}

/// The shared Konami VRC IRQ (identical design to VRC6): an up-counter from a
/// reloadable latch, in CPU-cycle mode or scanline mode (a 341/3-dot
/// prescaler). VRC4 splits the latch reload across two nibble writes.
#[derive(Clone, Serialize, Deserialize)]
struct VrcIrq {
    latch: u8,
    counter: u8,
    enabled: bool,
    enable_after_ack: bool,
    cycle_mode: bool,
    prescaler: i16,
    line: bool,
}

impl VrcIrq {
    fn new() -> Self {
        VrcIrq {
            latch: 0,
            counter: 0,
            enabled: false,
            enable_after_ack: false,
            cycle_mode: false,
            prescaler: 341,
            line: false,
        }
    }

    fn write_latch_lo(&mut self, val: u8) {
        self.latch = (self.latch & 0xF0) | (val & 0x0F);
    }

    fn write_latch_hi(&mut self, val: u8) {
        self.latch = (self.latch & 0x0F) | ((val & 0x0F) << 4);
    }

    fn write_control(&mut self, val: u8) {
        self.enable_after_ack = val & 1 != 0;
        self.enabled = val & 2 != 0;
        self.cycle_mode = val & 4 != 0;
        self.line = false;
        if self.enabled {
            self.counter = self.latch;
            self.prescaler = 341;
        }
    }

    fn ack(&mut self) {
        self.line = false;
        self.enabled = self.enable_after_ack;
    }

    fn clock(&mut self) {
        if !self.enabled {
            return;
        }
        if !self.cycle_mode {
            // Scanline mode: one step per 113.667 CPU cycles.
            self.prescaler -= 3;
            if self.prescaler > 0 {
                return;
            }
            self.prescaler += 341;
        }
        if self.counter == 0xFF {
            self.counter = self.latch;
            self.line = true;
        } else {
            self.counter += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vrc4(mapper: u8) -> Vrc4 {
        // 8 x 8KB PRG (64KB), 16 x 1KB CHR; each byte = its bank index.
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        Vrc4::new(mapper, prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn reg_scramble_mapper21() {
        let m = vrc4(21);
        // VRC4a A1,A2: index bit0=A1, bit1=A2.
        assert_eq!(m.reg(0x8000), (0, 0));
        assert_eq!(m.reg(0x8002), (0, 1)); // A1
        assert_eq!(m.reg(0x8004), (0, 2)); // A2
        assert_eq!(m.reg(0x8006), (0, 3)); // A1|A2
        // VRC4c A6,A7 reach the same indices.
        assert_eq!(m.reg(0x8040), (0, 1)); // A6
        assert_eq!(m.reg(0x8080), (0, 2)); // A7
        assert_eq!(m.reg(0x80C0), (0, 3)); // A6|A7
    }

    #[test]
    fn reg_scramble_mapper22_swapped() {
        let m = vrc4(22);
        // A1 -> bit0, A0 -> bit1 (swapped vs the natural order).
        assert_eq!(m.reg(0x8000), (0, 0));
        assert_eq!(m.reg(0x8001), (0, 2)); // A0 -> bit1
        assert_eq!(m.reg(0x8002), (0, 1)); // A1 -> bit0
        assert_eq!(m.reg(0x8003), (0, 3));
    }

    #[test]
    fn reg_scramble_mapper23() {
        let m = vrc4(23);
        // A0 -> bit0, A1 -> bit1 (natural order).
        assert_eq!(m.reg(0x8001), (0, 1)); // A0
        assert_eq!(m.reg(0x8002), (0, 2)); // A1
        assert_eq!(m.reg(0x8003), (0, 3));
        // A2,A3 reach the same indices (OR trick).
        assert_eq!(m.reg(0x8004), (0, 1)); // A2 -> bit0
        assert_eq!(m.reg(0x8008), (0, 2)); // A3 -> bit1
        assert_eq!(m.reg(0x800C), (0, 3));
        assert_eq!(m.reg(0xF000), (7, 0));
    }

    #[test]
    fn reg_scramble_mapper25_swapped() {
        let m = vrc4(25);
        // SWAPPED vs mapper 23: A1 -> bit0, A0 -> bit1.
        assert_eq!(m.reg(0x8002), (0, 1)); // A1 -> bit0
        assert_eq!(m.reg(0x8001), (0, 2)); // A0 -> bit1
        assert_eq!(m.reg(0x8003), (0, 3));
        // A3,A2 reach the same indices (OR trick, also swapped).
        assert_eq!(m.reg(0x8008), (0, 1)); // A3 -> bit0
        assert_eq!(m.reg(0x8004), (0, 2)); // A2 -> bit1
        assert_eq!(m.reg(0x800C), (0, 3));
        assert_eq!(m.reg(0xF000), (7, 0));
    }

    #[test]
    fn prg_switch_and_fixed_banks() {
        let mut m = vrc4(23);
        m.cpu_write(0x8000, 3); // $8000 -> bank 3
        m.cpu_write(0xA000, 5); // $A000 -> bank 5
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xA000), 5);
        assert_eq!(m.cpu_read(0xC000), 6); // fixed second-to-last
        assert_eq!(m.cpu_read(0xE000), 7); // fixed last
    }

    #[test]
    fn prg_swap_mode() {
        let mut m = vrc4(23);
        m.cpu_write(0x8000, 3);
        // $9002 bit1 enables swap: $C000 becomes the swappable window, $8000
        // becomes the fixed second-to-last bank.
        m.cpu_write(0x9002, 0x02); // index 2 via A1 (group 1)
        assert_eq!(m.cpu_read(0x8000), 6); // now fixed second-to-last
        assert_eq!(m.cpu_read(0xC000), 3); // now swappable
        assert_eq!(m.cpu_read(0xE000), 7); // last stays fixed
    }

    #[test]
    fn chr_nibble_assembly() {
        let mut m = vrc4(23);
        // Slot 0 lives at group 3 ($B000), index 0 (low) and 1 (high).
        m.cpu_write(0xB000, 0x0A); // low nibble
        m.cpu_write(0xB001, 0x01); // high nibble -> bank 0x1A = 26 % 16 = 10
        assert_eq!(m.ppu_read(0x0000), 26 % 16);
        // Slot 7 lives at group 6 ($E000), index 2 (low) and 3 (high).
        m.cpu_write(0xE002, 0x03); // index 2 via A1 -> low nibble of slot 7
        m.cpu_write(0xE003, 0x00); // index 3 -> high nibble 0 -> bank 3
        assert_eq!(m.ppu_read(0x1C00), 3);
    }

    #[test]
    fn vrc2_chr_granularity_quirk() {
        // Mapper 22 right-shifts the programmed CHR bank by one.
        let mut m = vrc4(22);
        m.cpu_write(0xB000, 0x04); // low nibble = 4
        m.cpu_write(0xB001, 0x00); // high nibble = 0 -> programmed 4, shifted -> 2
        assert_eq!(m.ppu_read(0x0000), 2);
    }

    #[test]
    fn mirroring_vrc4_four_way() {
        let mut m = vrc4(23);
        m.cpu_write(0x9000, 0);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0x9000, 1);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0x9000, 2);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        m.cpu_write(0x9000, 3);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
    }

    #[test]
    fn mirroring_vrc2_two_way() {
        let mut m = vrc4(22);
        m.cpu_write(0x9000, 1);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0x9000, 0);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        // Only bit 0 matters: a value of 2 is "even" -> Vertical.
        m.cpu_write(0x9000, 2);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn four_screen_never_overridden() {
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        let mut m = Vrc4::new(23, prg, chr, Mirroring::FourScreen);
        m.cpu_write(0x9000, 1);
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
    }

    #[test]
    fn irq_scanline_mode_cadence() {
        let mut m = vrc4(23);
        // Latch $FF via two nibble writes: IRQ on first counter step.
        m.cpu_write(0xF000, 0x0F); // latch low nibble
        m.cpu_write(0xF001, 0x0F); // latch high nibble -> 0xFF
        m.cpu_write(0xF002, 0x02); // enable, scanline mode
        // First step lands after ceil(341/3) = 114 CPU cycles.
        for _ in 0..113 {
            m.cpu_clock();
        }
        assert!(!m.irq());
        m.cpu_clock();
        assert!(m.irq());
        // $F003 acks.
        m.cpu_write(0xF003, 0); // index 3 (ack) via A0+A1 (group 7)
        assert!(!m.irq());
    }

    #[test]
    fn irq_cycle_mode_countdown() {
        // Mapper 25 swaps the two selector lines, so the IRQ sub-registers move:
        // latch-high is index 1 (A1 -> $F002) and control is index 2 (A0 -> $F001).
        let mut m = vrc4(25);
        m.cpu_write(0xF000, 0x0D); // latch low: 0x0D
        m.cpu_write(0xF002, 0x0F); // latch high: 0xFD -> 3 steps to 0xFF
        m.cpu_write(0xF001, 0x06); // enable, cycle mode
        for _ in 0..2 {
            m.cpu_clock();
            assert!(!m.irq());
        }
        m.cpu_clock();
        assert!(m.irq());
    }

    #[test]
    fn vrc2_has_no_irq() {
        let mut m = vrc4(22);
        // IRQ register writes are ignored; clocking never raises the line.
        m.cpu_write(0xF000, 0x0F);
        m.cpu_write(0xF001, 0x0F);
        m.cpu_write(0xF002, 0x06);
        for _ in 0..1000 {
            m.cpu_clock();
        }
        assert!(!m.irq());
    }
}
