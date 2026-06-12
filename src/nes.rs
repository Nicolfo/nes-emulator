use crate::bus::Bus;
use crate::cartridge::{load_rom, Region};
use crate::cpu::Cpu;

pub struct Nes {
    pub cpu: Cpu,
    region: Region,
}

impl Nes {
    pub fn new(rom: &[u8]) -> Result<Self, String> {
        let (cart, region) = load_rom(rom)?;
        let mut cpu = Cpu::new(Bus::with_region(cart, region));
        cpu.reset();
        Ok(Nes { cpu, region })
    }

    pub fn region(&self) -> Region {
        self.region
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

    /// Set the host audio output rate (resets the APU filter chain).
    pub fn set_sample_rate(&mut self, hz: f64) {
        self.cpu.bus.apu.set_sample_rate(hz);
    }

    /// Nudge the resampling ratio for dynamic rate control.
    pub fn tune_audio(&mut self, hz: f64) {
        self.cpu.bus.apu.tune(hz);
    }

    /// Drain audio samples generated since the last call.
    pub fn take_audio(&mut self) -> Vec<f32> {
        self.cpu.bus.apu.take_samples()
    }
}
