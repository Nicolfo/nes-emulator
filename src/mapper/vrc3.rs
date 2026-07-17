use super::{Mapper, Mirroring};
use serde::{Deserialize, Serialize};

/// VRC3 (mapper 73, Salamander): 16KB switchable PRG at $8000 with the last
/// 16KB fixed at $C000, 8KB CHR RAM (no banking), 8KB PRG RAM at $6000, and a
/// 16-bit (or 8-bit-selectable) IRQ counter clocked every CPU cycle. The board
/// has no mapper-controlled mirroring; it is fixed by solder pads / header.
#[derive(Serialize, Deserialize)]
pub struct Vrc3 {
    #[serde(skip)]
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    mirroring: Mirroring,
    prg_bank: u8,
    irq: Vrc3Irq,
}

impl Vrc3 {
    pub fn new(prg: Vec<u8>, chr: Vec<u8>, mirroring: Mirroring) -> Self {
        // VRC3 boards carry 8KB of CHR RAM; ignore any CHR ROM in the header.
        let _ = chr;
        Vrc3 {
            prg,
            chr: vec![0; 0x2000],
            prg_ram: vec![0; 0x2000],
            mirroring,
            prg_bank: 0,
            irq: Vrc3Irq::new(),
        }
    }
}

impl Mapper for Vrc3 {
    crate::impl_mapper_savestate!(chr, prg_ram);

    fn set_ram_sizes(&mut self, prg_ram: usize, chr_ram: usize) {
        if prg_ram > 0 {
            self.prg_ram = vec![0; prg_ram];
        }
        if chr_ram > 0 {
            self.chr = vec![0; chr_ram];
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        let banks = self.prg.len() / 0x4000;
        match addr {
            0x8000..=0xBFFF => {
                self.prg[(self.prg_bank as usize % banks) * 0x4000 + (addr as usize & 0x3FFF)]
            }
            0xC000..=0xFFFF => self.prg[(banks - 1) * 0x4000 + (addr as usize & 0x3FFF)],
            _ => 0,
        }
    }

    fn cpu_write(&mut self, addr: u16, val: u8) {
        match addr & 0xF000 {
            0x8000 => self.irq.set_latch_nibble(0, val),
            0x9000 => self.irq.set_latch_nibble(1, val),
            0xA000 => self.irq.set_latch_nibble(2, val),
            0xB000 => self.irq.set_latch_nibble(3, val),
            0xC000 => self.irq.write_control(val),
            0xD000 => self.irq.ack(),
            // $F000-$FFFF: low bits select the 16KB PRG bank.
            0xF000 => self.prg_bank = val,
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.chr[(addr as usize) & 0x1FFF]
    }

    fn ppu_write(&mut self, addr: u16, val: u8) {
        self.chr[(addr as usize) & 0x1FFF] = val;
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
        self.irq.clock();
    }
}

/// The Konami VRC3 IRQ: an up-counter clocked every CPU cycle. In 16-bit mode
/// the full counter is reloaded from the 16-bit latch on overflow past $FFFF;
/// in 8-bit mode only the low byte counts and is reloaded from the low latch
/// byte on overflow past $FF. Acknowledge copies the "enable after ack" bit
/// back into the enable bit.
#[derive(Serialize, Deserialize)]
struct Vrc3Irq {
    latch: u16,
    counter: u16,
    enabled: bool,
    enable_after_ack: bool,
    eight_bit: bool,
    line: bool,
}

impl Vrc3Irq {
    fn new() -> Self {
        Vrc3Irq {
            latch: 0,
            counter: 0,
            enabled: false,
            enable_after_ack: false,
            eight_bit: false,
            line: false,
        }
    }

    /// Sets one of the four 4-bit groups of the 16-bit latch (group 0 = bits
    /// 0-3, group 3 = bits 12-15). Only the low nibble of `val` is used.
    fn set_latch_nibble(&mut self, group: u8, val: u8) {
        let shift = group as u16 * 4;
        let mask = 0x000F << shift;
        self.latch = (self.latch & !mask) | (((val as u16) & 0x0F) << shift);
    }

    fn write_control(&mut self, val: u8) {
        self.enable_after_ack = val & 1 != 0;
        self.enabled = val & 2 != 0;
        self.eight_bit = val & 4 != 0;
        self.line = false;
        // Enabling reloads the counter from the full 16-bit latch.
        if self.enabled {
            self.counter = self.latch;
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
        if self.eight_bit {
            // Only the low 8 bits count; on overflow past $FF reload the low
            // byte from the low latch byte (high byte left untouched).
            if (self.counter & 0xFF) == 0xFF {
                self.counter = (self.counter & 0xFF00) | (self.latch & 0x00FF);
                self.line = true;
            } else {
                self.counter = (self.counter & 0xFF00) | ((self.counter + 1) & 0x00FF);
            }
        } else if self.counter == 0xFFFF {
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

    fn vrc3() -> Vrc3 {
        // 8 x 16KB PRG (128KB); each byte = its 16KB bank index. CHR RAM.
        let prg: Vec<u8> = (0..8 * 0x4000).map(|i| (i / 0x4000) as u8).collect();
        Vrc3::new(prg, vec![], Mirroring::Vertical)
    }

    #[test]
    fn prg_switchable_8000_fixed_c000() {
        let mut m = vrc3();
        assert_eq!(m.cpu_read(0x8000), 0);
        assert_eq!(m.cpu_read(0xC000), 7); // fixed last bank
        m.cpu_write(0xF000, 3);
        assert_eq!(m.cpu_read(0x8000), 3);
        assert_eq!(m.cpu_read(0xC000), 7); // still fixed
    }

    #[test]
    fn chr_ram_rw() {
        let mut m = vrc3();
        m.ppu_write(0x1234, 0xAB);
        assert_eq!(m.ppu_read(0x1234), 0xAB);
    }

    #[test]
    fn prg_ram_rw() {
        let mut m = vrc3();
        assert_eq!(m.prg_ram_read(0x6000), Some(0));
        m.prg_ram_mut().unwrap()[0] = 0x42;
        assert_eq!(m.prg_ram_read(0x6000), Some(0x42));
        assert_eq!(m.prg_ram().unwrap()[0], 0x42);
    }

    #[test]
    fn mirroring_fixed_from_header() {
        let m = vrc3();
        assert_eq!(m.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn irq_16bit_overflow_and_ack() {
        let mut m = vrc3();
        // Latch = $FFFD via the four nibble registers: 3 cycles to overflow.
        m.cpu_write(0x8000, 0x0D); // bits 0-3
        m.cpu_write(0x9000, 0x0F); // bits 4-7
        m.cpu_write(0xA000, 0x0F); // bits 8-11
        m.cpu_write(0xB000, 0x0F); // bits 12-15
        m.cpu_write(0xC000, 0x02); // enable, 16-bit mode -> counter = $FFFD
        for i in 0..2 {
            m.cpu_clock();
            assert!(!m.irq(), "IRQ too early at cycle {i}");
        }
        m.cpu_clock(); // $FFFF -> overflow -> IRQ, reload to $FFFD
        assert!(m.irq());
        // Acknowledge with enable-after-ack clear: line drops and counter halts.
        m.cpu_write(0xD000, 0);
        assert!(!m.irq());
        for _ in 0..10_000 {
            m.cpu_clock();
        }
        assert!(!m.irq());
    }

    #[test]
    fn irq_16bit_reloads_from_latch() {
        let mut m = vrc3();
        m.cpu_write(0x8000, 0x0F); // latch low nibble
        m.cpu_write(0xC000, 0x03); // enable + enable-after-ack, 16-bit
        // counter = $000F; needs ($FFFF - $000F) + 1 cycles to overflow.
        let steps = (0xFFFFu32 - 0x000F) + 1;
        for _ in 0..steps {
            m.cpu_clock();
        }
        assert!(m.irq());
        // Ack re-enables (enable-after-ack set) and the counter resumes from
        // the reloaded latch value $000F.
        m.cpu_write(0xD000, 0);
        assert!(!m.irq());
        for _ in 0..steps {
            m.cpu_clock();
        }
        assert!(m.irq());
    }

    #[test]
    fn irq_8bit_mode_overflow_and_reload() {
        let mut m = vrc3();
        // Low latch byte = $FE -> 2 cycles to overflow past $FF in 8-bit mode.
        m.cpu_write(0x8000, 0x0E); // bits 0-3
        m.cpu_write(0x9000, 0x0F); // bits 4-7  -> low byte $FE
        // A high nibble that must NOT be touched by the 8-bit reload.
        m.cpu_write(0xA000, 0x0A); // bits 8-11
        m.cpu_write(0xB000, 0x0A); // bits 12-15 -> high byte $AA
        m.cpu_write(0xC000, 0x06); // enable + 8-bit mode -> counter = $AAFE
        m.cpu_clock(); // $AAFF
        assert!(!m.irq());
        m.cpu_clock(); // low byte overflows -> IRQ, low byte reloads to $FE
        assert!(m.irq());
        // Ack clears the line, then another 2-cycle cadence fires again,
        // confirming the low byte (not the full counter) was reloaded.
        m.cpu_write(0xD000, 0); // ack; control bit0 was 0 -> stays disabled
        assert!(!m.irq());
        m.cpu_write(0xC000, 0x06); // re-enable, 8-bit; counter = latch $AAFE
        m.cpu_clock();
        assert!(!m.irq());
        m.cpu_clock();
        assert!(m.irq());
    }
}
