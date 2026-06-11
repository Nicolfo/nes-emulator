use crate::controller::Controller;
use crate::mapper::Mapper;
use crate::ppu::Ppu;

pub struct Bus {
    ram: [u8; 0x800],
    pub ppu: Ppu,
    pub cart: Box<dyn Mapper>,
    pub controller1: Controller,
    nmi_line: bool,
    dma_stall: u64,
    pub cycles: u64,
}

impl Bus {
    pub fn new(cart: Box<dyn Mapper>) -> Self {
        Bus {
            ram: [0; 0x800],
            ppu: Ppu::new(),
            cart,
            controller1: Controller::default(),
            nmi_line: false,
            dma_stall: 0,
            cycles: 0,
        }
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize],
            0x2000..=0x3FFF => self.ppu.read_register(addr & 7, &mut *self.cart),
            0x4015 => 0, // APU status (not emulated)
            0x4016 => self.controller1.read(),
            0x4017 => 0x40, // controller 2 not connected
            0x4000..=0x401F => 0,
            _ => self.cart.cpu_read(addr),
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize] = val,
            0x2000..=0x3FFF => self.ppu.write_register(addr & 7, val, &mut *self.cart),
            0x4014 => self.oam_dma(val),
            0x4016 => self.controller1.write(val),
            0x4000..=0x401F => {} // APU registers ignored
            _ => self.cart.cpu_write(addr, val),
        }
    }

    fn oam_dma(&mut self, page: u8) {
        let base = (page as u16) << 8;
        for i in 0..256u16 {
            let v = self.read(base + i);
            let dst = self.ppu.oam_addr_for_dma().wrapping_add(i as u8);
            self.ppu.oam[dst as usize] = v;
        }
        self.dma_stall = 513 + (self.cycles & 1);
    }

    /// Advance the PPU 3 dots per CPU cycle; latch NMI edge.
    pub fn tick(&mut self, cpu_cycles: u64) {
        self.cycles += cpu_cycles;
        for _ in 0..cpu_cycles * 3 {
            self.ppu.tick(&mut *self.cart);
        }
        if self.ppu.take_nmi() {
            self.nmi_line = true;
        }
    }

    pub fn take_nmi(&mut self) -> bool {
        std::mem::take(&mut self.nmi_line)
    }

    pub fn take_dma_stall(&mut self) -> u64 {
        std::mem::take(&mut self.dma_stall)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::{Mirroring, Nrom};

    fn bus() -> Bus {
        Bus::new(Box::new(Nrom::new(vec![0; 0x8000], vec![0; 0x2000], Mirroring::Vertical)))
    }

    #[test]
    fn ram_mirroring() {
        let mut b = bus();
        b.write(0x0000, 0x42);
        assert_eq!(b.read(0x0800), 0x42);
        assert_eq!(b.read(0x1800), 0x42);
    }

    #[test]
    fn dma_copies_and_stalls() {
        let mut b = bus();
        for i in 0..256u16 {
            b.write(0x0200 + i, i as u8);
        }
        b.write(0x4014, 0x02);
        assert_eq!(b.ppu.oam[0], 0);
        assert_eq!(b.ppu.oam[255], 255);
        let stall = b.take_dma_stall();
        assert!(stall == 513 || stall == 514);
    }
}
