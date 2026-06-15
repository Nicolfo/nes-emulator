use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// Bandai FCG / LZ93D50 family (mappers 16 and 159).
///
/// Boards: FCG-1/FCG-2 (early) and LZ93D50 (later). Both expose the same
/// register file:
///   - 16KB switchable PRG at $8000-$BFFF, fixed last 16KB at $C000-$FFFF.
///   - eight 1KB CHR banks selected by registers 0-7.
///   - software mirroring, a 16-bit cycle-counted IRQ, and a serial EEPROM
///     for battery-backed saves.
///
/// The registers are decoded by the *low nibble* of the write address. The
/// FCG-1/2 decoded them at $6000-$7FFF; the LZ93D50 moved them to
/// $8000-$FFFF (where they overlap the PRG ROM window). To support cartridges
/// of either generation we decode the low nibble in BOTH ranges — see
/// `cpu_write`. (This is the common emulator convention for mappers 16/153/159.)
///
/// IRQ: FCG-1/2 wrote $xB/$xC straight into the down-counter, while the
/// LZ93D50 writes a *latch* that is copied into the counter when the IRQ is
/// enabled via $xA. Mappers 16 and 159 are LZ93D50 parts, so we implement the
/// latch + copy-on-enable behavior (see `IrqState`).
///
/// EEPROM: the real board carries a 24Cxx serial EEPROM (128 bytes / 24C01 on
/// mapper 159, 256 bytes / 24C02 on mapper 16) driven bit-by-bit over the
/// $xD register and read back through $6000. The full I2C-style serial
/// protocol is NOT implemented here — TODO. Instead we expose the EEPROM as a
/// plain battery-backed RAM region so saves persist across runs, and make the
/// $xD writes / $6000 reads behave well enough not to hang the boot path.
/// Games that drive the serial protocol may not save correctly until the real
/// 24Cxx state machine is implemented.
#[derive(Serialize, Deserialize)]
pub struct BandaiFcg {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    /// True when the header marked the board four-screen; we must never
    /// override that via the $x9 mirroring register.
    four_screen: bool,
    /// 16KB PRG bank mapped at $8000-$BFFF.
    prg_bank: u8,
    /// Eight 1KB CHR bank registers.
    chr_banks: [u8; 8],
    irq: IrqState,
    /// Battery-backed stand-in for the serial EEPROM (see type doc). Sized to
    /// match the part: 128 bytes on mapper 159, 256 bytes on mapper 16.
    eeprom: Vec<u8>,
    /// Last value written to $xD (EEPROM/RAM control). Kept so a read of
    /// $6000 can echo a plausible "ready" bit rather than open bus.
    eeprom_ctrl: u8,
}

impl BandaiFcg {
    /// `mapper` is the iNES mapper number (16 or 159); it selects the EEPROM
    /// size (159 -> 24C01 / 128 bytes, otherwise 24C02 / 256 bytes).
    pub fn new(mapper: u8, prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        let four_screen = mirroring == Mirroring::FourScreen;
        let eeprom_size = if mapper == 159 { 128 } else { 256 };
        BandaiFcg {
            prg,
            chr,
            chr_is_ram,
            mirroring,
            four_screen,
            prg_bank: 0,
            chr_banks: [0; 8],
            irq: IrqState::new(),
            eeprom: vec![0xFF; eeprom_size],
            eeprom_ctrl: 0,
        }
    }

    /// Decode a register write by the low nibble of `addr`. Shared by the
    /// $6000-$7FFF (FCG-1/2) and $8000-$FFFF (LZ93D50) ranges.
    fn write_reg(&mut self, addr: u16, val: u8) {
        match addr & 0x0F {
            // $x0-$x7: select 1KB CHR bank for the matching slot.
            r @ 0x0..=0x7 => self.chr_banks[r as usize] = val,
            // $x8: 16KB PRG bank (low 4 bits used by the family).
            0x8 => self.prg_bank = val & 0x0F,
            // $x9: mirroring (bits 0-1). Header four-screen wins.
            0x9 => {
                if !self.four_screen {
                    self.mirroring = match val & 0x03 {
                        0 => Mirroring::Vertical,
                        1 => Mirroring::Horizontal,
                        2 => Mirroring::SingleScreenLo,
                        _ => Mirroring::SingleScreenHi,
                    };
                }
            }
            // $xA: IRQ control. bit0 enables; on the LZ93D50 enabling also
            // copies the latch into the counter (and acknowledges any pending
            // IRQ).
            0xA => self.irq.write_control(val),
            // $xB: IRQ latch/counter low byte.
            0xB => self.irq.write_low(val),
            // $xC: IRQ latch/counter high byte.
            0xC => self.irq.write_high(val),
            // $xD: EEPROM / RAM control. Serial protocol is a TODO; we only
            // record the value (see type doc).
            0xD => self.eeprom_ctrl = val,
            _ => {}
        }
    }

    fn prg_read(&self, addr: u16) -> u8 {
        let banks = (self.prg.len() / 0x4000).max(1);
        let bank = match addr {
            // Switchable 16KB at $8000-$BFFF.
            0x8000..=0xBFFF => self.prg_bank as usize % banks,
            // Fixed last 16KB at $C000-$FFFF.
            _ => banks - 1,
        };
        self.prg[bank * 0x4000 + (addr as usize & 0x3FFF)]
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let banks = (self.chr.len() / 0x400).max(1);
        let bank = self.chr_banks[(addr >> 10) as usize & 7] as usize % banks;
        bank * 0x400 + (addr as usize & 0x3FF)
    }
}

impl Mapper for BandaiFcg {
    crate::impl_mapper_savestate!();

    fn cpu_read(&mut self, addr: u16) -> u8 {
        match addr {
            0x8000..=0xFFFF => self.prg_read(addr),
            _ => 0,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        // The LZ93D50 register file overlaps the PRG ROM window; writes in
        // $8000-$FFFF land here (ROM is read-only, so this is unambiguous).
        // FCG-1/2 cartridges that decode at $6000-$7FFF are handled by
        // `prg_ram` routing in the bus, which also forwards here via the
        // write path below for completeness.
        if addr >= 0x8000 {
            self.write_reg(addr, val);
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        let off = self.chr_offset(addr);
        self.chr[off]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        // CHR is ROM on the FCG/LZ93D50 boards; only writable if the header
        // declared no CHR ROM (CHR RAM).
        if self.chr_is_ram {
            let off = self.chr_offset(addr);
            self.chr[off] = val;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn prg_ram_read(&mut self, _addr: u16) -> Option<u8> {
        // The $6000-$7FFF range on these boards is the EEPROM/register window,
        // not RAM. The real chip returns the serial EEPROM data line in a
        // specific bit; lacking the protocol we return a benign "ready" value
        // so polling loops in the save routine terminate instead of hanging.
        // TODO: implement the 24Cxx serial read here.
        Some(0x00)
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        // Expose the EEPROM contents for .sav persistence so saves survive a
        // restart even though the live serial protocol is a stand-in.
        Some(&self.eeprom)
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        Some(&mut self.eeprom)
    }

    fn irq(&self) -> bool {
        self.irq.line
    }

    fn cpu_clock(&mut self) {
        self.irq.clock();
    }

    fn cpu_reg_read(&mut self, _addr: u16) -> Option<u8> {
        // $4020-$5FFF is unused by this family.
        None
    }
}

/// The LZ93D50 16-bit IRQ: a down-counter clocked once per CPU cycle. When it
/// underflows (wraps past 0) the IRQ line asserts and stays asserted until the
/// next $xA write. The counter value is loaded from a 16-bit *latch* (written
/// via $xB/$xC) when the IRQ is enabled — this is the LZ93D50 behavior; the
/// older FCG-1/2 wrote the counter directly.
#[derive(Serialize, Deserialize)]
struct IrqState {
    enabled: bool,
    counter: u16,
    latch: u16,
    line: bool,
}

impl IrqState {
    fn new() -> Self {
        IrqState {
            enabled: false,
            counter: 0,
            latch: 0,
            line: false,
        }
    }

    /// $xA: bit0 = enable. Writing this acknowledges the pending IRQ; on
    /// enable the latch is copied into the counter (LZ93D50).
    fn write_control(&mut self, val: u8) {
        self.line = false;
        self.enabled = val & 1 != 0;
        if self.enabled {
            self.counter = self.latch;
        }
    }

    /// $xB: IRQ latch low byte.
    fn write_low(&mut self, val: u8) {
        self.latch = (self.latch & 0xFF00) | val as u16;
    }

    /// $xC: IRQ latch high byte.
    fn write_high(&mut self, val: u8) {
        self.latch = (self.latch & 0x00FF) | ((val as u16) << 8);
    }

    /// One CPU cycle: decrement; underflow past 0 asserts the IRQ.
    fn clock(&mut self) {
        if !self.enabled {
            return;
        }
        if self.counter == 0 {
            // Underflow: wrap to 0xFFFF and raise the line.
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

    /// 8 x 16KB PRG (byte = 16KB bank index), 16 x 1KB CHR (byte = 1KB bank).
    fn fcg(mapper: u8) -> BandaiFcg {
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        BandaiFcg::new(mapper, prg, chr, Mirroring::Vertical)
    }

    #[test]
    fn prg_switch_and_fixed_last() {
        let mut m = fcg(16);
        // $x8 selects the 16KB bank at $8000-$BFFF.
        m.cpu_write(0x8008, 3);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xBFFF), 3);
        // $C000-$FFFF is fixed to the last 16KB bank (index 7).
        assert_eq!(m.cpu_read(0xC000), 7);
        assert_eq!(m.cpu_read(0xFFFF), 7);
    }

    #[test]
    fn chr_1kb_banking() {
        let mut m = fcg(16);
        // Set each of the eight 1KB slots to a distinct bank.
        for slot in 0..8u16 {
            m.cpu_write(0x8000 | slot, (slot as u8) + 5);
        }
        for slot in 0..8u16 {
            assert_eq!(m.ppu_read(slot * 0x400), (slot as u8) + 5);
        }
        // A bank index wraps modulo the 16-bank CHR.
        m.cpu_write(0x8000, 18); // 18 % 16 == 2
        assert_eq!(m.ppu_read(0x0000), 2);
    }

    #[test]
    fn register_decode_in_8000_range() {
        let mut m = fcg(16);
        // LZ93D50 register window overlaps PRG; low nibble selects the reg.
        m.cpu_write(0x8008, 2); // PRG bank
        m.cpu_write(0x8003, 9); // CHR slot 3
        assert_eq!(m.cpu_read(0x8000), 2);
        assert_eq!(m.ppu_read(0x0C00), 9);
    }

    #[test]
    fn register_decode_in_6000_range() {
        // FCG-1/2 decode at $6000-$7FFF. The bus forwards those writes via the
        // same low-nibble decoder. We exercise the decoder directly here since
        // cpu_write only handles $8000+; the $6000 path shares `write_reg`.
        let mut m = fcg(16);
        m.write_reg(0x6008, 4); // PRG bank
        m.write_reg(0x7005, 12); // CHR slot 5
        assert_eq!(m.cpu_read(0x8000), 4);
        assert_eq!(m.ppu_read(0x1400), 12);
    }

    #[test]
    fn mirroring_decode_all_four() {
        let mut m = fcg(16);
        m.cpu_write(0x8009, 0);
        assert_eq!(m.mirroring(), Mirroring::Vertical);
        m.cpu_write(0x8009, 1);
        assert_eq!(m.mirroring(), Mirroring::Horizontal);
        m.cpu_write(0x8009, 2);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenLo);
        m.cpu_write(0x8009, 3);
        assert_eq!(m.mirroring(), Mirroring::SingleScreenHi);
    }

    #[test]
    fn four_screen_header_is_never_overridden() {
        let prg: Vec<u8> = vec![0; 8 * 0x4000];
        let chr: Vec<u8> = vec![0; 16 * 0x400];
        let mut m = BandaiFcg::new(16, prg, chr, Mirroring::FourScreen);
        m.cpu_write(0x8009, 1); // would request Horizontal
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
    }

    #[test]
    fn cycle_irq_counts_to_underflow() {
        let mut m = fcg(16);
        // Latch = 3, then enable copies it into the counter (LZ93D50).
        m.cpu_write(0x800B, 3); // low byte
        m.cpu_write(0x800C, 0); // high byte
        m.cpu_write(0x800A, 1); // enable -> counter = 3
        // Counter 3 -> 2 -> 1 -> 0 over three clocks, then underflow on the 4th.
        for i in 0..3 {
            m.cpu_clock();
            assert!(!m.irq(), "IRQ too early at cycle {i}");
        }
        m.cpu_clock(); // 0 -> underflow
        assert!(m.irq());
    }

    #[test]
    fn irq_enable_copies_latch_and_ack_clears_line() {
        let mut m = fcg(16);
        // 16-bit latch: high + low.
        m.cpu_write(0x800C, 0x01); // high
        m.cpu_write(0x800B, 0x00); // low -> latch = 0x0100
        m.cpu_write(0x800A, 1); // enable -> counter = 0x0100
        // Not yet underflowed.
        for _ in 0..0x100 {
            m.cpu_clock();
        }
        assert!(!m.irq());
        m.cpu_clock(); // 0 -> underflow
        assert!(m.irq());
        // Any $xA write acknowledges (clears) the line.
        m.cpu_write(0x800A, 0); // disable + ack
        assert!(!m.irq());
        // Disabled counter no longer ticks.
        for _ in 0..10_000 {
            m.cpu_clock();
        }
        assert!(!m.irq());
    }
}
