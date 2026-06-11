use crate::apu::Apu;
use crate::controller::Controller;
use crate::mapper::Mapper;
use crate::ppu::Ppu;

/// RDY assertion delay (CPU cycles) for a DMC "load" DMA after a $4015 write.
/// Overridable via NES_DMC_LOAD_DELAY for timing experiments.
pub fn dmc_load_delay() -> u8 {
    static DELAY: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *DELAY.get_or_init(|| {
        std::env::var("NES_DMC_LOAD_DELAY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2)
    })
}

/// Parity of bus cycles on which the DMC fetch ("get" cycle) may occur.
/// Overridable via NES_DMC_GET_PARITY.
pub fn dmc_get_parity() -> u64 {
    static PARITY: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *PARITY.get_or_init(|| {
        std::env::var("NES_DMC_GET_PARITY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1)
    })
}

pub struct Bus {
    pub ram: [u8; 0x800],
    pub ppu: Ppu,
    pub apu: Apu,
    pub cart: Box<dyn Mapper>,
    pub controller1: Controller,
    pub cycles: u64,
    pub open_bus: u8,
    /// DMC sample fetch requested by the APU; serviced by the CPU's DMA logic.
    pub dmc_request: Option<u16>,
    /// Cycles until the DMC DMA may halt the CPU (models RDY assertion delay
    /// after a $4015 load).
    pub dmc_delay: u8,
    /// OAM DMA page latched by a $4014 write; serviced by the CPU.
    pub oam_dma_page: Option<u8>,
    /// Debug: log writes to this RAM address as (cycle, value).
    pub watch_addr: Option<u16>,
    pub watch_log: Vec<(u64, u8)>,
    /// Debug: log reads of this address as (value, scanline, dot).
    pub read_watch_addr: Option<u16>,
    pub read_watch_log: Vec<(u8, i16, u16)>,
}

impl Bus {
    pub fn new(cart: Box<dyn Mapper>) -> Self {
        Bus {
            ram: [0; 0x800],
            ppu: Ppu::new(),
            apu: Apu::new(),
            cart,
            controller1: Controller::default(),
            cycles: 0,
            open_bus: 0,
            dmc_request: None,
            dmc_delay: 0,
            oam_dma_page: None,
            watch_addr: None,
            watch_log: Vec::new(),
            read_watch_addr: None,
            read_watch_log: Vec::new(),
        }
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => {
                let r = self.ram[(addr & 0x07FF) as usize];
                self.open_bus = r;
                r
            }
            0x2000..=0x3FFF => {
                let r = self.ppu.read_register(addr & 7, &mut *self.cart);
                if self.read_watch_addr == Some(addr) {
                    self.read_watch_log
                        .push((r, self.ppu.scanline, self.ppu.dot));
                }
                self.open_bus = r;
                r
            }
            // $4015 is internal to the 2A03: reading it does not drive the data bus.
            0x4015 => {
                let apu_val = self.apu.read_status();
                (apu_val & 0xDF) | (self.open_bus & 0x20)
            }
            // Controller reads drive D0-D4 only; D5-D7 stay open bus.
            0x4016 => {
                let ctrl_val = self.controller1.read();
                let r = (ctrl_val & 0x1F) | (self.open_bus & 0xE0);
                self.open_bus = r;
                r
            }
            0x4017 => {
                let ctrl_val = 0x00; // controller 2 not connected
                let r = (ctrl_val & 0x1F) | (self.open_bus & 0xE0);
                self.open_bus = r;
                r
            }
            0x4000..=0x401F => self.open_bus,
            0x4020..=0x7FFF => self.open_bus,
            _ => {
                let r = self.cart.cpu_read(addr);
                self.open_bus = r;
                r
            }
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        if let Some(w) = self.watch_addr && addr >= w && addr < w + 16 {
                self.watch_log.push((self.cycles, val));
        }
        self.open_bus = val;
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize] = val,
            0x2000..=0x3FFF => self.ppu.write_register(addr & 7, val, &mut *self.cart),
            0x4014 => self.oam_dma_page = Some(val),
            0x4016 => self.controller1.write(val),
            0x4015 => {
                self.apu.write(addr, val);
                // Disabling the DMC aborts a pending DMA before its fetch.
                if val & 0x10 == 0 && self.dmc_request.is_some() {
                    self.dmc_request = None;
                    self.apu.dmc_abort_fetch();
                }
            }
            0x4000..=0x4013 | 0x4017 => self.apu.write(addr, val),
            0x4018..=0x401F => {}
            _ => self.cart.cpu_write(addr, val),
        }
    }

    /// Read performed by the OAM DMA unit. The APU registers ($4000-$401F)
    /// only respond when the 6502's own address bus selects them, so a DMA
    /// read of that range sees open bus unless the halted CPU's address is
    /// also in range.
    pub fn read_for_dma(&mut self, addr: u16, cpu_addr_in_apu: bool) -> u8 {
        if (0x4000..=0x5FFF).contains(&addr) {
            if cpu_addr_in_apu {
                // With the registers active, they are mirrored every $20 bytes.
                self.read(0x4000 | (addr & 0x1F))
            } else {
                self.open_bus
            }
        } else {
            self.read(addr)
        }
    }

    /// First part of a CPU cycle: APU 1 step, PPU 2 dots. The CPU's bus
    /// access samples after this, mid-cycle.
    pub fn tick_cycle_pre(&mut self) {
        self.cycles += 1;
        if self.dmc_request.is_some() && self.dmc_delay > 0 {
            self.dmc_delay -= 1;
        }
        if let Some((addr, load)) = self.apu.tick() {
            self.dmc_request = Some(addr);
            if load {
                self.dmc_delay = dmc_load_delay();
            }
        }
        for _ in 0..2 {
            self.ppu.tick(&mut *self.cart);
        }
    }

    /// Rest of the CPU cycle (PPU 1 dot); interrupt lines are polled after.
    pub fn tick_cycle_post(&mut self) {
        self.ppu.tick(&mut *self.cart);
        // Controller strobe latches button state on "put" cycles (odd here).
        if self.cycles & 1 == 1 {
            self.controller1.clock_put_cycle();
        }
    }

    /// Advance one full CPU cycle with no bus access.
    pub fn tick_cycle(&mut self) {
        self.tick_cycle_pre();
        self.tick_cycle_post();
    }

    /// Level of the PPU's NMI output (VBlank flag AND NMI enable).
    pub fn nmi_line(&self) -> bool {
        self.ppu.nmi_line()
    }

    /// Advance the PPU alone by `n` dots (sets the CPU/PPU clock alignment).
    pub fn tick_ppu_dots(&mut self, n: u32) {
        for _ in 0..n {
            self.ppu.tick(&mut *self.cart);
        }
    }

    /// Level-triggered IRQ line (APU frame counter and DMC).
    pub fn irq_asserted(&self) -> bool {
        self.apu.irq()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::{Mirroring, Nrom};

    fn bus() -> Bus {
        Bus::new(Box::new(Nrom::new(
            vec![0; 0x8000],
            vec![0; 0x2000],
            Mirroring::Vertical,
        )))
    }

    #[test]
    fn ram_mirroring() {
        let mut b = bus();
        b.write(0x0000, 0x42);
        assert_eq!(b.read(0x0800), 0x42);
        assert_eq!(b.read(0x1800), 0x42);
    }

    #[test]
    fn oam_dma_write_latches_page() {
        let mut b = bus();
        b.write(0x4014, 0x02);
        assert_eq!(b.oam_dma_page, Some(0x02));
    }
}
