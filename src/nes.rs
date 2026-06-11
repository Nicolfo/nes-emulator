use crate::bus::Bus;
use crate::cartridge::load_rom;
use crate::cpu::Cpu;

pub struct Nes {
    pub cpu: Cpu,
}

impl Nes {
    pub fn new(rom: &[u8]) -> Result<Self, String> {
        let cart = load_rom(rom)?;
        let mut cpu = Cpu::new(Bus::new(cart));
        cpu.reset();
        Ok(Nes { cpu })
    }

    /// Run until the PPU enters vblank (one full frame).
    pub fn run_frame(&mut self) {
        self.cpu.bus.ppu.frame_complete = false;
        while !self.cpu.bus.ppu.frame_complete {
            self.cpu.step();
        }
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.cpu.bus.ppu.framebuffer
    }
}
