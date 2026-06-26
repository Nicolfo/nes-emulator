use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Irem H3001 (mapper 65), used by games such as Daiku no Gen-san 2 and
/// Spartan X 2.
///
/// PRG: two switchable 8KB registers plus two fixed banks, arranged by the
/// `$9000` mode bit (nesdev mapper 65):
/// - `$8000` register (reg 0) and `$A000` register (reg 1) hold the switchable
///   banks; `$A000-$BFFF` is always reg 1.
/// - `$9000` bit 7 = 0: `$8000-$9FFF` = reg 0, `$C000-$DFFF` = fixed `$3E`.
/// - `$9000` bit 7 = 1: `$8000-$9FFF` = fixed `$3E`, `$C000-$DFFF` = reg 0.
/// - `$E000-$FFFF` is always the fixed `$3F` bank. `$3E`/`$3F` mask to the last
///   two 8KB banks of the ROM.
///
/// CHR: eight 1KB banks selected by `$B000-$B007`, one register per 1KB slot
/// across `$0000-$1FFF`.
///
/// Mirroring: `$9001` bits 7-6 - %00=Vertical, %10=Horizontal, %01/%11=1-screen.
///
/// IRQ: a 16-bit down-counter clocked once per CPU cycle. When enabled it
/// decrements every cycle and asserts IRQ when it reaches 0, then *stops* at 0
/// (no auto-reload). `$9005`/`$9006` set the high/low bytes of the reload
/// latch, `$9004` copies the latch into the live counter (and acks), and
/// `$9003` bit 7 enables the counter (and acks).
#[derive(Clone, Serialize, Deserialize)]
pub struct H3001 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    four_screen: bool,
    // Switchable 8KB PRG registers: reg 0 ($8000 write) and reg 1 ($A000).
    prg_reg0: u8,
    prg_reg1: u8,
    // $9000 bit 7: false -> reg0 at $8000 / $C000 fixed; true -> the reverse.
    prg_mode: bool,
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
            prg_reg0: 0,
            prg_reg1: 0,
            prg_mode: false,
            chr_banks: [0; 8],
            irq: H3001Irq::new(),
        }
    }

    /// Map a CPU address ($8000-$FFFF) to a PRG ROM offset (8KB granularity).
    fn prg_offset(&self, addr: u16) -> usize {
        let banks = (self.prg.len() / 0x2000).max(1);
        let last = banks - 1; // $3F
        let second = banks.saturating_sub(2); // $3E
        let reg0 = self.prg_reg0 as usize % banks;
        let reg1 = self.prg_reg1 as usize % banks;
        let bank = match (addr >> 13) & 3 {
            // $8000-$9FFF: reg 0, or the fixed $3E bank in the swapped mode.
            0 => {
                if self.prg_mode {
                    second
                } else {
                    reg0
                }
            }
            1 => reg1, // $A000-$BFFF: always reg 1
            // $C000-$DFFF: the fixed $3E bank, or reg 0 in the swapped mode.
            2 => {
                if self.prg_mode {
                    reg0
                } else {
                    second
                }
            }
            _ => last, // $E000-$FFFF: fixed $3F
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
    crate::impl_mapper_savestate!(chr_is_ram = chr_is_ram);

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.prg[self.prg_offset(addr)]
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr {
            0x8000 => self.prg_reg0 = val,
            0xA000 => self.prg_reg1 = val,
            // $9000 bit 7 selects which window reg 0 drives.
            0x9000 => self.prg_mode = val & 0x80 != 0,
            0x9001 => {
                // Bits 7-6: %00=Vertical, %10=Horizontal, %01/%11=1-screen.
                // Four-screen boards ignore the mapper's mirroring control.
                if !self.four_screen {
                    self.mirroring = match (val >> 6) & 3 {
                        0 => Mirroring::Vertical,
                        2 => Mirroring::Horizontal,
                        _ => Mirroring::SingleScreenLo,
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
#[derive(Clone, Serialize, Deserialize)]
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
        if !self.enabled || self.counter == 0 {
            // Disabled, or already fired and stopped at 0.
            return;
        }
        self.counter -= 1;
        if self.counter == 0 {
            // Reaching 0 asserts the IRQ; the counter then stays at 0 until a
            // reload ($9004) or another enable.
            self.line = true;
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
    fn prg_layout_modes() {
        // 8 banks (0..7): fixed $3E = second-last (6), $3F = last (7).
        let mut m = h3001();
        m.cpu_write(0x8000, 2); // reg 0
        m.cpu_write(0xA000, 5); // reg 1
        // Mode 0: $8000=reg0, $A000=reg1, $C000=$3E (6), $E000=$3F (7).
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.cpu_read(0x9FFF), 2);
        assert_eq!(m.cpu_read(0xA000), 5);
        assert_eq!(m.cpu_read(0xC000), 6);
        assert_eq!(m.cpu_read(0xE000), 7);
        assert_eq!(m.cpu_read(0xFFFF), 7);
        // Mode 1 ($9000 bit 7): $8000=$3E (6), $C000=reg0 (2).
        m.cpu_write(0x9000, 0x80);
        assert_eq!(m.cpu_read(0x8000), 6);
        assert_eq!(m.cpu_read(0xA000), 5);
        assert_eq!(m.cpu_read(0xC000), 2);
        assert_eq!(m.cpu_read(0xE000), 7);
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
        // 3 -> 2 -> 1, no IRQ; the clock that reaches 0 asserts it.
        for i in 0..2 {
            m.cpu_clock();
            assert!(!m.irq(), "IRQ too early at cycle {i}");
        }
        m.cpu_clock(); // 1 -> 0 asserts
        assert!(m.irq());
    }

    #[test]
    fn irq_16bit_latch_high_low() {
        let mut m = h3001();
        m.cpu_write(0x9005, 0x01); // high byte
        m.cpu_write(0x9006, 0x00); // low byte -> latch = 0x0100 = 256
        m.cpu_write(0x9004, 0); // reload
        m.cpu_write(0x9003, 0x80); // enable
        // 255 decrements stay above 0; the 256th reaches 0 and asserts.
        for _ in 0..255 {
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
        m.cpu_write(0x9006, 0x02); // latch = 2
        m.cpu_write(0x9004, 0); // counter = 2
        m.cpu_write(0x9003, 0x80); // enable
        m.cpu_clock(); // 2 -> 1
        assert!(!m.irq());
        m.cpu_clock(); // 1 -> 0 asserts
        assert!(m.irq());

        // $9004 acknowledges (and reloads the counter from the latch).
        m.cpu_write(0x9004, 0);
        assert!(!m.irq());

        // Counter reloaded to 2; two clocks fire it again.
        m.cpu_clock();
        m.cpu_clock();
        assert!(m.irq());
        m.cpu_write(0x9003, 0x00); // disable + ack
        assert!(!m.irq());
        for _ in 0..1000 {
            m.cpu_clock();
        }
        assert!(!m.irq(), "disabled counter must not fire");
    }
}
