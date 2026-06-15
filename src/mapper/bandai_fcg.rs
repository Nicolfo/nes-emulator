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
/// The registers are decoded by the *low nibble* of the write address, but the
/// range differs by board: the FCG-1/2 (mapper 16 submapper 4) decodes them at
/// $6000-$7FFF, while the LZ93D50 (submapper 5, and mapper 159) decodes them at
/// $8000-$FFFF (overlapping the PRG ROM window). The submapper picks the range;
/// when it is unspecified (plain iNES, submapper 0) we decode BOTH ranges, the
/// behavior the NES 2.0 spec mandates for that case.
///
/// IRQ: the FCG-1/2 writes $xB/$xC straight into the down-counter, while the
/// LZ93D50 writes a *latch* copied into the counter when the IRQ is enabled via
/// $xA. `IrqState::immediate` selects which (see [`IrqState`]).
///
/// EEPROM: the LZ93D50 carries a 24Cxx serial EEPROM (128 bytes / 24C01 on
/// mapper 159 and submapper 1, 256 bytes / 24C02 on submapper 5) driven over
/// the two-wire interface - $xD bit 6 = SDA, bit 5 = SCL - and read back at
/// $6000 bit 4. The full I2C slave protocol is emulated in [`SerialEeprom`] and
/// persists to the .sav file. The bare FCG-1/2 has no EEPROM.
#[derive(Serialize, Deserialize)]
pub struct BandaiFcg {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    /// True when the header marked the board four-screen; we must never
    /// override that via the $x9 mirroring register.
    four_screen: bool,
    /// Decode registers in $6000-$7FFF (FCG-1/2) and/or $8000-$FFFF (LZ93D50).
    reg_lo: bool,
    reg_hi: bool,
    /// 16KB PRG bank mapped at $8000-$BFFF.
    prg_bank: u8,
    /// Eight 1KB CHR bank registers.
    chr_banks: [u8; 8],
    irq: IrqState,
    /// Serial EEPROM, present only on EEPROM-equipped LZ93D50 boards.
    eeprom: Option<SerialEeprom>,
}

impl BandaiFcg {
    /// `mapper` is the iNES number (16 or 159) and `submapper` the NES 2.0
    /// submapper (0 when unspecified). Together they pick the board: register
    /// decode range, IRQ latch behavior, and EEPROM type.
    pub fn new(
        mapper: u8,
        submapper: u8,
        prg: Vec<u8>,
        chr: Vec<u8>,
        mirroring: Mirroring,
    ) -> Self {
        let chr_is_ram = chr.is_empty();
        let chr = if chr_is_ram { vec![0; 0x2000] } else { chr };
        let four_screen = mirroring == Mirroring::FourScreen;

        // (reg_lo, reg_hi, immediate IRQ, EEPROM).
        let (reg_lo, reg_hi, immediate, eeprom) = match (mapper, submapper) {
            // FCG-1/2: $6000-$7FFF registers, counter written directly, no EEPROM.
            (16, 4) => (true, false, true, None),
            // LZ93D50 + 24C02: $8000-$FFFF, latched IRQ.
            (16, 5) => (false, true, false, Some(SerialEeprom::new_24c02())),
            // LZ93D50 + 24C01 (deprecated submapper 1; modern split is mapper 159).
            (16, 1) => (false, true, false, Some(SerialEeprom::new_24c01())),
            // Mapper 159 is the LZ93D50 + 24C01 split-off.
            (159, _) => (false, true, false, Some(SerialEeprom::new_24c01())),
            // Unspecified (submapper 0, plain iNES, or 2/3): emulate both ranges
            // with FCG-style immediate IRQ; keep a 24C02 so LZ93D50 saves work.
            _ => (true, true, true, Some(SerialEeprom::new_24c02())),
        };

        BandaiFcg {
            prg,
            chr,
            chr_is_ram,
            mirroring,
            four_screen,
            reg_lo,
            reg_hi,
            prg_bank: 0,
            chr_banks: [0; 8],
            irq: IrqState::new(immediate),
            eeprom,
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
            // $xD: serial EEPROM I/O. bit6 = SDA (data), bit5 = SCL (clock).
            0xD => {
                if let Some(ee) = self.eeprom.as_mut() {
                    ee.write_lines(val & 0x20 != 0, val & 0x40 != 0);
                }
            }
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
        // Decode register writes in whichever range(s) this board uses. The bus
        // forwards both $6000-$7FFF and $8000-$FFFF writes here.
        let in_lo = (0x6000..=0x7FFF).contains(&addr);
        let in_hi = addr >= 0x8000;
        if (in_lo && self.reg_lo) || (in_hi && self.reg_hi) {
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
        // EEPROM boards return the serial data-out line in bit 4; the bare
        // FCG-1/2 has nothing here (open bus).
        self.eeprom.as_ref().map(|ee| (ee.read_sda() as u8) << 4)
    }

    fn prg_ram(&self) -> Option<&[u8]> {
        // Expose the EEPROM cells for .sav persistence.
        self.eeprom.as_ref().map(|ee| ee.cells())
    }

    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
        self.eeprom.as_mut().map(|ee| ee.cells_mut())
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

/// The Bandai 16-bit IRQ: a down-counter clocked once per CPU cycle. When it
/// underflows (wraps past 0) the IRQ line asserts and stays asserted until the
/// next $xA write.
///
/// The two board generations differ in how $xB/$xC writes reach the counter:
/// the LZ93D50 writes a *latch* copied into the counter when the IRQ is enabled
/// via $xA, while the older FCG-1/2 writes the counter directly. `immediate`
/// selects the FCG-1/2 behavior.
#[derive(Serialize, Deserialize)]
struct IrqState {
    enabled: bool,
    immediate: bool,
    counter: u16,
    latch: u16,
    line: bool,
}

impl IrqState {
    fn new(immediate: bool) -> Self {
        IrqState {
            enabled: false,
            immediate,
            counter: 0,
            latch: 0,
            line: false,
        }
    }

    /// $xA: bit0 = enable. Writing this acknowledges the pending IRQ; on the
    /// LZ93D50 enabling also copies the latch into the counter.
    fn write_control(&mut self, val: u8) {
        self.line = false;
        self.enabled = val & 1 != 0;
        if self.enabled && !self.immediate {
            self.counter = self.latch;
        }
    }

    /// $xB: IRQ counter/latch low byte.
    fn write_low(&mut self, val: u8) {
        self.latch = (self.latch & 0xFF00) | val as u16;
        if self.immediate {
            self.counter = (self.counter & 0xFF00) | val as u16;
        }
    }

    /// $xC: IRQ counter/latch high byte.
    fn write_high(&mut self, val: u8) {
        self.latch = (self.latch & 0x00FF) | ((val as u16) << 8);
        if self.immediate {
            self.counter = (self.counter & 0x00FF) | ((val as u16) << 8);
        }
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

/// A two-wire (I2C) serial EEPROM as fitted to the Bandai LZ93D50: a 24C01
/// (128 bytes, mapper 159) or 24C02 (256 bytes, mapper 16). The CPU drives the
/// clock (SCL) and data (SDA) lines through $800D; the chip drives SDA low to
/// acknowledge and to shift out read data, sampled at $6000.
///
/// The two parts differ only in addressing: the 24C02 begins a transfer with a
/// `1010xxxR` device-select byte followed by a separate word-address byte,
/// while the 24C01 has no device byte - its first byte is the 7-bit word
/// address plus the R/W bit. Everything else (START/STOP detection, MSB-first
/// byte shifts, master/slave ACK handshakes, address auto-increment) is shared.
#[derive(Serialize, Deserialize)]
struct SerialEeprom {
    data: Vec<u8>,
    /// 24C02 (true) uses a device-select byte then a word-address byte; the
    /// 24C01 (false) takes the word address directly in the first byte.
    device_byte: bool,
    addr_mask: u8,
    scl: bool,
    sda: bool,
    /// SDA line the chip drives (true = released/high, false = pulled low).
    out: bool,
    phase: EePhase,
    /// Bits shifted so far in the current byte (0..=8).
    bits: u8,
    /// Byte being shifted in (receive) or out (transmit).
    shift: u8,
    /// Current word address (auto-increments after each byte).
    addr: u8,
    /// What the byte currently being received represents.
    rx: EeRx,
    /// True once the current transfer has been identified as a read (from the
    /// R/W bit in the device byte or, on the 24C01, the first address byte).
    read_pending: bool,
}

#[derive(Serialize, Deserialize, PartialEq, Clone, Copy)]
enum EePhase {
    /// Bus idle, waiting for a START.
    Idle,
    /// Shifting a byte in from the master.
    Receive,
    /// Driving SDA low for one clock to acknowledge a received byte.
    Ack,
    /// Shifting a byte out to the master.
    Transmit,
    /// Sampling the master's ACK/NAK after a transmitted byte.
    AckCheck,
}

#[derive(Serialize, Deserialize, PartialEq, Clone, Copy)]
enum EeRx {
    /// 24C02 device-select byte (`1010xxxR`).
    Device,
    /// Word-address byte.
    Word,
    /// Data byte to store.
    Data,
}

impl SerialEeprom {
    fn new_24c01() -> Self {
        Self::new(128, false)
    }

    fn new_24c02() -> Self {
        Self::new(256, true)
    }

    fn new(size: usize, device_byte: bool) -> Self {
        SerialEeprom {
            data: vec![0xFF; size],
            device_byte,
            addr_mask: (size - 1) as u8,
            scl: false,
            sda: false,
            out: true,
            phase: EePhase::Idle,
            bits: 0,
            shift: 0,
            addr: 0,
            // 24C01 has no device byte: the first received byte is the address.
            rx: if device_byte {
                EeRx::Device
            } else {
                EeRx::Word
            },
            read_pending: false,
        }
    }

    fn cells(&self) -> &[u8] {
        &self.data
    }

    fn cells_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Value seen by the CPU on the SDA line (true = high).
    fn read_sda(&self) -> bool {
        self.out
    }

    /// First receive state for a fresh transfer.
    fn first_rx(&self) -> EeRx {
        if self.device_byte {
            EeRx::Device
        } else {
            EeRx::Word
        }
    }

    /// Apply new SCL/SDA levels from a $800D write and advance the state
    /// machine. START/STOP are recognised by SDA transitions while SCL is high;
    /// data is shifted on SCL edges otherwise.
    fn write_lines(&mut self, scl: bool, sda: bool) {
        if self.scl && scl {
            // SCL steady high: a SDA edge is a START or STOP condition.
            if self.sda && !sda {
                // START: begin a new transfer.
                self.phase = EePhase::Receive;
                self.rx = self.first_rx();
                self.bits = 0;
                self.shift = 0;
                self.out = true;
            } else if !self.sda && sda {
                // STOP: release the bus.
                self.phase = EePhase::Idle;
                self.out = true;
            }
        } else if !self.scl && scl {
            self.on_rise(sda);
        } else if self.scl && !scl {
            self.on_fall();
        }
        self.scl = scl;
        self.sda = sda;
    }

    /// SCL rising edge: sample an incoming data/ack bit.
    fn on_rise(&mut self, sda: bool) {
        match self.phase {
            EePhase::Receive => {
                self.shift = (self.shift << 1) | sda as u8;
                self.bits += 1;
            }
            EePhase::AckCheck => {
                // Master ACK (SDA low) -> keep streaming; NAK (high) -> stop.
                if sda {
                    self.phase = EePhase::Idle;
                    self.out = true;
                } else {
                    self.addr = self.addr.wrapping_add(1) & self.addr_mask;
                    self.load_output();
                }
            }
            _ => {}
        }
    }

    /// SCL falling edge: complete a received byte, drive ACK/data, or shift the
    /// next outgoing bit.
    fn on_fall(&mut self) {
        match self.phase {
            EePhase::Receive if self.bits == 8 => {
                self.finish_received_byte();
            }
            EePhase::Ack => {
                // The ACK clock just ended; resume per the queued operation.
                self.out = true;
                self.bits = 0;
                self.shift = 0;
                let start_read = match self.rx {
                    // Device byte done: a read goes straight to shifting data
                    // out, a write waits for the word address.
                    EeRx::Device => {
                        if !self.read_pending {
                            self.rx = EeRx::Word;
                        }
                        self.read_pending
                    }
                    // Word address done: 24C01 reads start here too.
                    EeRx::Word => {
                        if !self.read_pending {
                            self.rx = EeRx::Data;
                        }
                        self.read_pending
                    }
                    // A data byte was stored; keep receiving more.
                    EeRx::Data => {
                        self.rx = EeRx::Data;
                        false
                    }
                };
                if start_read {
                    // We are on a falling edge, so present the first read bit
                    // now for the master to sample on the next rising edge.
                    self.load_output();
                    self.shift_out_bit();
                } else {
                    self.phase = EePhase::Receive;
                }
            }
            EePhase::Transmit => {
                if self.bits < 8 {
                    // Present the next bit (MSB first) for the master to sample
                    // on the following rising edge.
                    self.shift_out_bit();
                } else {
                    // Byte sent; release SDA and read the master's ACK/NAK.
                    self.out = true;
                    self.phase = EePhase::AckCheck;
                }
            }
            _ => {}
        }
    }

    /// Act on a fully received byte and drive the ACK pulse.
    fn finish_received_byte(&mut self) {
        match self.rx {
            EeRx::Device => {
                // `1010xxxR`: bit 0 is the read/write select.
                self.read_pending = self.shift & 1 != 0;
            }
            EeRx::Word => {
                if self.device_byte {
                    self.addr = self.shift & self.addr_mask;
                } else {
                    // 24C01: 7-bit address + R/W in bit 0.
                    self.read_pending = self.shift & 1 != 0;
                    self.addr = (self.shift >> 1) & self.addr_mask;
                }
            }
            EeRx::Data => {
                self.data[self.addr as usize] = self.shift;
                self.addr = self.addr.wrapping_add(1) & self.addr_mask;
            }
        }
        // Acknowledge: pull SDA low for the ACK clock.
        self.out = false;
        self.phase = EePhase::Ack;
    }

    /// Latch the byte at the current address for shifting out. The first bit is
    /// presented separately (always on a falling edge) via `shift_out_bit`.
    fn load_output(&mut self) {
        self.shift = self.data[self.addr as usize];
        self.bits = 0;
        self.phase = EePhase::Transmit;
    }

    /// Drive the next outgoing bit (MSB first) onto SDA.
    fn shift_out_bit(&mut self) {
        self.out = self.shift & 0x80 != 0;
        self.shift <<= 1;
        self.bits += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 8 x 16KB PRG (byte = 16KB bank index), 16 x 1KB CHR (byte = 1KB bank).
    fn fcg_sub(mapper: u8, submapper: u8) -> BandaiFcg {
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        let chr: Vec<u8> = (0..16 * 0x400).map(|i| (i / 0x400) as u8).collect();
        BandaiFcg::new(mapper, submapper, prg, chr, Mirroring::Vertical)
    }

    /// Default test board: LZ93D50 (submapper 5), registers at $8000-$FFFF.
    fn fcg(mapper: u8) -> BandaiFcg {
        fcg_sub(mapper, 5)
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
        // FCG-1/2 (submapper 4) decodes registers at $6000-$7FFF via cpu_write.
        let mut m = fcg_sub(16, 4);
        m.cpu_write(0x6008, 4); // PRG bank
        m.cpu_write(0x7005, 12); // CHR slot 5
        assert_eq!(m.cpu_read(0x8000), 4);
        assert_eq!(m.ppu_read(0x1400), 12);
        // The bare FCG-1/2 has no EEPROM: $6000 reads are open bus.
        assert_eq!(m.prg_ram_read(0x6000), None);
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
        let mut m = BandaiFcg::new(16, 5, prg, chr, Mirroring::FourScreen);
        m.cpu_write(0x8009, 1); // would request Horizontal
        assert_eq!(m.mirroring(), Mirroring::FourScreen);
    }

    #[test]
    fn fcg1_irq_writes_counter_directly() {
        // Submapper 4 (FCG-1/2): $xB/$xC write the counter immediately, with no
        // separate enable-time latch copy.
        let mut m = fcg_sub(16, 4);
        m.cpu_write(0x600A, 1); // enable (immediate mode: counter unchanged)
        m.cpu_write(0x600B, 2); // counter low = 2
        m.cpu_write(0x600C, 0); // counter high = 0 -> counter = 2
        m.cpu_clock(); // 2 -> 1
        assert!(!m.irq());
        m.cpu_clock(); // 1 -> 0
        assert!(!m.irq());
        m.cpu_clock(); // 0 -> underflow asserts
        assert!(m.irq());
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

    /// Bit-banging I2C master used to exercise the EEPROM the way a game would.
    struct I2c<'a> {
        e: &'a mut SerialEeprom,
    }

    impl I2c<'_> {
        fn set(&mut self, scl: bool, sda: bool) {
            self.e.write_lines(scl, sda);
        }
        fn start(&mut self) {
            self.set(true, true);
            self.set(true, false); // SDA falls while SCL high
            self.set(false, false);
        }
        fn stop(&mut self) {
            self.set(false, false);
            self.set(true, false);
            self.set(true, true); // SDA rises while SCL high
        }
        /// Send a byte MSB-first; returns true if the chip acknowledged.
        fn send(&mut self, byte: u8) -> bool {
            for i in (0..8).rev() {
                let bit = (byte >> i) & 1 != 0;
                self.set(false, bit);
                self.set(true, bit);
            }
            // Release SDA and clock the ACK; chip pulls low to acknowledge.
            self.set(false, true);
            self.set(true, true);
            let acked = !self.e.read_sda();
            self.set(false, true);
            acked
        }
        /// Read a byte MSB-first; `cont` drives the master ACK (true = keep going).
        fn recv(&mut self, cont: bool) -> u8 {
            let mut v = 0u8;
            self.set(false, true);
            for _ in 0..8 {
                self.set(true, true);
                v = (v << 1) | self.e.read_sda() as u8;
                self.set(false, true);
            }
            self.set(false, !cont);
            self.set(true, !cont);
            self.set(false, !cont);
            v
        }
    }

    #[test]
    fn eeprom_24c02_round_trip() {
        let mut e = SerialEeprom::new_24c02();
        // Write 0xA5 to word address 0x10.
        {
            let mut m = I2c { e: &mut e };
            m.start();
            assert!(m.send(0xA0)); // device select, write
            assert!(m.send(0x10)); // word address
            assert!(m.send(0xA5)); // data
            m.stop();
        }
        // Random read of word address 0x10.
        let mut m = I2c { e: &mut e };
        m.start();
        assert!(m.send(0xA0)); // device select, write (to set the address)
        assert!(m.send(0x10));
        m.start(); // repeated START
        assert!(m.send(0xA1)); // device select, read
        assert_eq!(m.recv(false), 0xA5); // NAK ends the read
        m.stop();
    }

    #[test]
    fn eeprom_24c01_round_trip() {
        let mut e = SerialEeprom::new_24c01();
        // 24C01: first byte is (addr << 1) | rw, no device-select byte.
        {
            let mut m = I2c { e: &mut e };
            m.start();
            assert!(m.send(0x10 << 1)); // address 0x10, write
            assert!(m.send(0x5A)); // data
            m.stop();
        }
        let mut m = I2c { e: &mut e };
        m.start();
        assert!(m.send((0x10 << 1) | 1)); // address 0x10, read
        assert_eq!(m.recv(false), 0x5A);
        m.stop();
    }

    #[test]
    fn eeprom_auto_increment_sequential_read() {
        let mut e = SerialEeprom::new_24c02();
        {
            let mut m = I2c { e: &mut e };
            m.start();
            assert!(m.send(0xA0));
            assert!(m.send(0x20)); // start address
            assert!(m.send(0x11));
            assert!(m.send(0x22));
            assert!(m.send(0x33)); // auto-increment writes
            m.stop();
        }
        let mut m = I2c { e: &mut e };
        m.start();
        assert!(m.send(0xA0));
        assert!(m.send(0x20));
        m.start();
        assert!(m.send(0xA1));
        assert_eq!(m.recv(true), 0x11); // ACK -> continue
        assert_eq!(m.recv(true), 0x22);
        assert_eq!(m.recv(false), 0x33); // NAK -> stop
        m.stop();
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
