use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Irem H3001 (mapper 65), used by games such as Daiku no Gen-san 2 and
/// Spartan X 2.
///
/// PRG: three independently switchable 8KB banks plus a fixed last bank.
/// - `$8000` selects the 8KB bank visible at CPU `$8000-$9FFF`.
/// - `$A000` selects the 8KB bank visible at CPU `$A000-$BFFF`.
/// - `$C000` selects the 8KB bank visible at CPU `$C000-$DFFF`.
/// - `$E000-$FFFF` is hardwired to the last 8KB of PRG.
///
/// (nesdev's summary describes a two-register layout where `$9000` bit 0
/// chooses whether `$8000` or `$C000` is the swappable window; the bank value
/// at `$E000` is `$3F`. This driver implements the simpler and more widely
/// used three-swappable-bank form per the task spec, with the last bank fixed
/// rather than register `$3F`.)
///
/// CHR: eight 1KB banks selected by `$B000-$B007`, one register per 1KB slot
/// across `$0000-$1FFF`.
///
/// Mirroring: `$9001`. Per nesdev the field is bits 7-6 (%00=Vert, %10=Horz,
/// %01/%11=1-screen). The common single-bit decode (bit 7: 1=Horizontal,
/// 0=Vertical) covers the games that use this mapper, so we decode bit 7 only.
///
/// IRQ: a 16-bit down-counter clocked once per CPU cycle. When enabled it
/// decrements every cycle and asserts IRQ when it underflows past 0 (i.e. when
/// a count of 0 wraps to 0xFFFF). `$9005`/`$9006` set the high/low bytes of the
/// reload latch, `$9004` copies the latch into the live counter (and acks),
/// and `$9003` bit 7 enables the counter (and acks).
#[derive(Serialize, Deserialize)]
pub struct H3001 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    four_screen: bool,
    // 8KB PRG banks for $8000, $A000, $C000. $E000 is the fixed last bank.
    prg_banks: [u8; 3],
    // 1KB CHR banks for the eight slots across $0000-$1FFF.
    chr_banks: [u8; 8],
    irq: H3001Irq,
}

impl H3001 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        let four_screen = mirroring == Mirroring::FourScreen;
        H3001 {
            prg,
            chr,
            chr_is_ram,
            mirroring,
            four_screen,
            prg_banks: [0; 3],
            chr_banks: [0; 8],
            irq: H3001Irq::new(),
        }
    }

    /// Map a CPU address ($8000-$FFFF) to a PRG ROM offset (8KB granularity).
    fn prg_offset(&self, addr: u16) -> usize {
        let banks = (self.prg.len() / 0x2000).max(1);
        let bank = match (addr >> 13) & 3 {
            0 => self.prg_banks[0] as usize % banks, // $8000-$9FFF
            1 => self.prg_banks[1] as usize % banks, // $A000-$BFFF
            2 => self.prg_banks[2] as usize % banks, // $C000-$DFFF
            _ => banks - 1,                          // $E000-$FFFF: fixed last
        };
        bank * 0x2000 + (addr as usize & 0x1FFF)
    }

    /// Map a PPU address ($0000-$1FFF) to a CHR offset (1KB granularity).
    fn chr_offset(&self, addr: u16) -> usize {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = (addr >> 10) as usize & 7;
        let bank = self.chr_banks[slot] as usize % banks;
        bank * 0x400 + (addr as usize & 0x3FF)
    }
}

impl Mapper for H3001 {
    crate::impl_mapper_savestate!();

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x8000 => self.prg_banks[0] = val,
            0xA000 => self.prg_banks[1] = val,
            0xC000 => self.prg_banks[2] = val,
            0x9001 => {
                // Bit 7: 1 = Horizontal, 0 = Vertical. Four-screen boards
                // ignore the mapper's mirroring control entirely.
                if !self.four_screen {
                    self.mirroring = if val & 0x80 != 0 {
                        Mirroring::Horizontal
                    } else {
                        Mirroring::Vertical
                    };
                }
            }
            0x9003 => self.irq.write_enable(val),
            0x9004 => self.irq.reload(),
            0x9005 => self.irq.set_latch_high(val),
            0x9006 => self.irq.set_latch_low(val),
            0xB000..=0xB007 => self.chr_banks[(addr & 7) as usize] = val,
            _ => {}
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

    fn irq(&self) -> bool {
        self.irq.line
    }

    fn cpu_clock(&mut self) {
        self.irq.clock();
    }
}

/// The H3001 IRQ: a 16-bit down-counter clocked once per CPU cycle.
///
/// `$9005`/`$9006` load the high/low byte of the reload latch. `$9004` copies
/// the latch into the live counter. `$9003` bit 7 enables/disables counting.
/// Both `$9003` and `$9004` writes acknowledge (clear) a pending IRQ. When
/// enabled, the counter decrements every cycle and asserts IRQ when it
/// underflows past 0 (a count of 0 wrapping to 0xFFFF).
#[derive(Serialize, Deserialize)]
struct H3001Irq {
    latch: u16,
    counter: u16,
    enabled: bool,
    line: bool,
}

impl H3001Irq {
    fn new() -> Self {
        H3001Irq {
            latch: 0,
            counter: 0,
            enabled: false,
            line: false,
        }
    }

    fn set_latch_high(&mut self, val: u8) {
        self.latch = (self.latch & 0x00FF) | ((val as u16) << 8);
    }

    fn set_latch_low(&mut self, val: u8) {
        self.latch = (self.latch & 0xFF00) | val as u16;
    }

    /// $9004: reload the counter from the latch and acknowledge.
    fn reload(&mut self) {
        self.counter = self.latch;
        self.line = false;
    }

    /// $9003: bit 7 enables the counter; the write also acknowledges.
    fn write_enable(&mut self, val: u8) {
        self.enabled = val & 0x80 != 0;
        self.line = false;
    }

    fn clock(&mut self) {
        if !self.enabled {
            return;
        }
        if self.counter == 0 {
            // Underflow past 0: wrap to 0xFFFF and assert IRQ.
            self.counter = 0xFFFF;
            self.line = true;
        } else {
            self.counter -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h3001() -> H3001 {
        // 8 x 8KB PRG, 8 x 1KB CHR; each byte equals its bank index.
        let prg: Vec<u8> = (0..8 * 0x2000).map(|i| (i / 0x2000) as u8).collect();
        let chr: Vec<u8> = (0..8 * 0x400).map(|i| (i / 0x400) as u8).collect();
        H3001::new(prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_three_banks_and_fixed_last() {
        let mut m = h3001();
        m.cpu_write(0x8000, 2);
        m.cpu_write(0xA000, 5);
        m.cpu_write(0xC000, 1);
        assert_eq!(m.cpu_read(0x8000), 2); // $8000 window
        assert_eq!(m.cpu_read(0x9FFF), 2);
        assert_eq!(m.cpu_read(0xA000), 5); // $A000 window
        assert_eq!(m.cpu_read(0xC000), 1); // $C000 window
        assert_eq!(m.cpu_read(0xE000), 7); // fixed last (8 banks -> index 7)
        assert_eq!(m.cpu_read(0xFFFF), 7);
    }

    #[test]
    fn prg_bank_wraps_modulo_count() {
        let mut m = h3001();
        m.cpu_write(0x8000, 8 + 3); // 8 banks: wraps to bank 3
        assert_eq!(m.cpu_read(0x8000), 3);
    }

    #[test]
    fn chr_1kb_banking() {
        let mut m = h3001();
        for slot in 0..8u16 {
            m.cpu_write(0xB000 + slot, (7 - slot) as u8);
        }
        for slot in 0..8u16 {
            let addr = slot * 0x400;
            assert_eq!(m.ppu_read(addr), (7 - slot) as u8);
            assert_eq!(m.ppu_read(addr + 0x3FF), (7 - slot) as u8);
        }
    }

    #[test]
    fn mirroring_bit7() {
        let mut m = h3001();
        m.cpu_write(0x9001, 0x80);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0x9001, 0x00);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn four_screen_is_locked() {
        let prg: Vec<u8> = vec![0; 8 * 0x2000];
        let chr: Vec<u8> = vec![0; 8 * 0x400];
        let mut m = H3001::new(prg, chr, Mirroring::FourScreen);
        m.cpu_write(0x9001, 0x80);
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
    }

    #[test]
    fn irq_counts_down_to_underflow() {
        let mut m = h3001();
        // Latch = 3, reload, enable.
        m.cpu_write(0x9005, 0x00); // high
        m.cpu_write(0x9006, 0x03); // low -> latch = 3
        m.cpu_write(0x9004, 0); // reload counter from latch
        m.cpu_write(0x9003, 0x80); // enable
        // 3 -> 2 -> 1 -> 0, none assert IRQ yet.
        for i in 0..3 {
            m.cpu_clock();
            assert!(!m.irq(), "IRQ too early at cycle {i}");
        }
        // Counter is now 0; next clock underflows and asserts.
        m.cpu_clock();
        assert!(m.irq());
    }

    #[test]
    fn irq_16bit_latch_high_low() {
        let mut m = h3001();
        m.cpu_write(0x9005, 0x01); // high byte
        m.cpu_write(0x9006, 0x00); // low byte -> latch = 0x0100 = 256
        m.cpu_write(0x9004, 0); // reload
        m.cpu_write(0x9003, 0x80); // enable
        // 256 decrements reach 0; 257th underflows.
        for _ in 0..256 {
            m.cpu_clock();
            assert!(!m.irq());
        }
        m.cpu_clock();
        assert!(m.irq());
    }

    #[test]
    fn irq_enable_and_ack() {
        let mut m = h3001();
        m.cpu_write(0x9005, 0x00);
        m.cpu_write(0x9006, 0x01); // latch = 1
        m.cpu_write(0x9004, 0); // counter = 1
        m.cpu_write(0x9003, 0x80); // enable
        m.cpu_clock(); // 1 -> 0
        assert!(!m.irq());
        m.cpu_clock(); // 0 -> underflow, assert
        assert!(m.irq());

        // $9004 acknowledges (and reloads).
        m.cpu_write(0x9004, 0);
        assert!(!m.irq());

        // $9003 also acknowledges; disabling stops the counter.
        m.cpu_write(0x9003, 0x80); // re-enable to fire again
        for _ in 0..2 {
            m.cpu_clock();
        }
        assert!(m.irq());
        m.cpu_write(0x9003, 0x00); // disable + ack
        assert!(!m.irq());
        for _ in 0..1000 {
            m.cpu_clock();
        }
        assert!(!m.irq(), "disabled counter must not fire");
    }
}
