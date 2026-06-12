use crate::bus::Bus;
use crate::cartridge::{Region, load_rom};
use crate::cpu::Cpu;

pub struct Nes {
    pub cpu: Cpu,
    region: Region,
    battery: bool,
}

impl Nes {
    pub fn new(rom: &[u8]) -> Result<Self, String> {
        let (cart, region, battery) = load_rom(rom)?;
        let mut cpu = Cpu::new(Bus::with_region(cart, region));
        cpu.reset();
        Ok(Nes {
            cpu,
            region,
            battery,
        })
    }

    pub fn region(&self) -> Region {
        self.region
    }

    /// Battery-backed PRG RAM to persist as a .sav file; None when the
    /// cartridge has no battery or the board has no PRG RAM.
    pub fn battery_ram(&self) -> Option<&[u8]> {
        if self.battery {
            self.cpu.bus.cart.prg_ram()
        } else {
            None
        }
    }

    /// Restore a previously saved .sav file into battery-backed PRG RAM.
    /// Ignored when the cartridge has no battery; size mismatches copy the
    /// overlapping prefix.
    pub fn load_battery_ram(&mut self, data: &[u8]) {
        if !self.battery {
            return;
        }
        if let Some(ram) = self.cpu.bus.cart.prg_ram_mut() {
            let n = ram.len().min(data.len());
            ram[..n].copy_from_slice(&data[..n]);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal 32KB iNES image; flags6 sets the mapper low nibble plus
    /// mirroring/battery/trainer bits.
    fn rom(flags6: u8) -> Vec<u8> {
        let mut data = vec![0u8; 16 + 32 * 1024 + 8 * 1024];
        data[0..4].copy_from_slice(b"NES\x1A");
        data[4] = 2; // 2x 16KB PRG
        data[5] = 1; // 8KB CHR
        data[6] = flags6;
        data
    }

    #[test]
    fn battery_ram_roundtrip() {
        // mapper 1 (MMC1) with the battery bit set
        let mut nes = Nes::new(&rom(0x12)).unwrap();
        nes.load_battery_ram(&[0xAB; 0x2000]);
        let ram = nes.battery_ram().unwrap();
        assert_eq!(ram.len(), 0x2000);
        assert!(ram.iter().all(|&b| b == 0xAB));
    }

    #[test]
    fn no_battery_means_no_sav() {
        // same board, battery bit clear
        let mut nes = Nes::new(&rom(0x10)).unwrap();
        nes.load_battery_ram(&[0xAB; 0x2000]); // ignored
        assert!(nes.battery_ram().is_none());
        assert_eq!(nes.cpu.bus.cart.prg_ram().unwrap()[0], 0);
    }
}
