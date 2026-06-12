use crate::apu::Apu;
use crate::cartridge::Region;
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
            .unwrap_or(1)
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
    region: Region,
    /// PAL runs 3.2 PPU dots per CPU cycle: phase counter for the extra dot
    /// every fifth cycle.
    pal_phase: u8,
    pub open_bus: u8,
    /// The CPU's internal data bus: latched on every CPU read/write cycle,
    /// but NOT by a DMC DMA sample fetch (the CPU is halted). Bit 5 of a
    /// $4015 read comes from here, not from the external bus.
    pub internal_bus: u8,
    /// DMC sample fetch requested by the APU; serviced by the CPU's DMA logic.
    pub dmc_request: Option<u16>,
    /// The pending DMC request was raised inside the disable grace window:
    /// it steals a single halt cycle, then aborts without fetching.
    pub dmc_ghost: bool,
    /// The pending DMC request is the retry of an attempt blocked by the
    /// $4015 enable pipeline: the DMA skips its alignment cycle (3 cycles).
    pub dmc_skip_align: bool,
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
        Self::with_region(cart, Region::Ntsc)
    }

    pub fn with_region(cart: Box<dyn Mapper>, region: Region) -> Self {
        let mut ppu = Ppu::new();
        ppu.set_region(region);
        let mut apu = Apu::new();
        apu.set_region(region);
        Bus {
            ram: [0; 0x800],
            ppu,
            apu,
            cart,
            controller1: Controller::default(),
            cycles: 0,
            region,
            pal_phase: 0,
            open_bus: 0,
            internal_bus: 0,
            dmc_request: None,
            dmc_ghost: false,
            dmc_skip_align: false,
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
                (apu_val & 0xDF) | (self.internal_bus & 0x20)
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
            0x4020..=0x5FFF => self.open_bus,
            0x6000..=0x7FFF => match self.cart.prg_ram_read(addr) {
                Some(v) => {
                    self.open_bus = v;
                    v
                }
                None => self.open_bus,
            },
            _ => {
                let r = self.cart.cpu_read(addr);
                self.open_bus = r;
                r
            }
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        if let Some(w) = self.watch_addr
            && addr >= w
            && addr < w + 16
        {
            self.watch_log.push((self.cycles, val));
        }
        self.open_bus = val;
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize] = val,
            0x2000..=0x3FFF => self.ppu.write_register(addr & 7, val, &mut *self.cart),
            0x4014 => self.oam_dma_page = Some(val),
            0x4016 => self.controller1.write(val),
            0x4000..=0x4013 | 0x4015 | 0x4017 => self.apu.write(addr, val),
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
            let v = self.read(addr);
            // With the registers active, a DMA read whose low 5 address bits
            // select $4015 returns the internal status register even when the
            // source drives the bus ($4015 never drives the external bus);
            // only its undriven bit 5 shows the external byte. $4016/$4017
            // lose the conflict against a driven source and stay invisible.
            if cpu_addr_in_apu && addr & 0x1F == 0x15 {
                (self.apu.read_status() & !0x20) | (v & 0x20)
            } else {
                // $4016 mirrors still clock the controller shift register,
                // even though the driven source wins the bus conflict.
                if cpu_addr_in_apu && addr & 0x1F == 0x16 {
                    self.controller1.read();
                }
                v
            }
        }
    }

    /// First part of a CPU cycle: APU 1 step, PPU 2 dots. The CPU's bus
    /// access samples after this, mid-cycle.
    pub fn tick_cycle_pre(&mut self) {
        self.cycles += 1;
        self.cart.cpu_clock();
        if self.dmc_request.is_some() && self.dmc_delay > 0 {
            self.dmc_delay -= 1;
        }
        if let Some((addr, load, ghost, skip_align)) = self.apu.tick(self.cart.audio_sample()) {
            self.dmc_request = Some(addr);
            self.dmc_ghost = ghost;
            self.dmc_skip_align = skip_align;
            if load {
                // Load DMA: RDY asserts a fixed delay after the (grid-aligned)
                // request, so the DMA is always 3 cycles.
                self.dmc_delay = dmc_load_delay();
            }
            if std::env::var("NES_DMA_LOG").is_ok() {
                eprintln!(
                    "cyc {} RAISE load={} ghost={} skip_align={}",
                    self.cycles, load, ghost, skip_align
                );
            }
        }
        for _ in 0..2 {
            self.ppu.tick(&mut *self.cart);
        }
    }

    /// Rest of the CPU cycle (PPU 1 dot); interrupt lines are polled after.
    pub fn tick_cycle_post(&mut self) {
        self.ppu.tick(&mut *self.cart);
        // PAL: 3.2 dots per CPU cycle — one extra dot every fifth cycle.
        if self.region == Region::Pal {
            self.pal_phase += 1;
            if self.pal_phase == 5 {
                self.pal_phase = 0;
                self.ppu.tick(&mut *self.cart);
            }
        }
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

    /// Level-triggered IRQ line (APU frame counter, DMC, and cartridge).
    pub fn irq_asserted(&self) -> bool {
        self.apu.irq() || self.cart.irq()
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
