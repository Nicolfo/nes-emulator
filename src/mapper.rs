mod axrom;
mod cnrom;
mod colordreams;
mod fme7;
mod gxrom;
mod mmc1;
mod mmc2;
mod mmc3;
mod nrom;
mod uxrom;

pub use axrom::Axrom;
pub use cnrom::Cnrom;
pub use colordreams::ColorDreams;
pub use fme7::Fme7;
pub use gxrom::Gxrom;
pub use mmc1::Mmc1;
pub use mmc2::Mmc2;
pub use mmc3::Mmc3;
pub use nrom::Nrom;
pub use uxrom::Uxrom;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirroring {
    Horizontal,
    Vertical,
    SingleScreenLo,
    SingleScreenHi,
}

pub trait Mapper {
    fn cpu_read(&mut self, addr: u16) -> u8;
    fn cpu_write(&mut self, addr: u16, val: u8);
    fn ppu_read(&mut self, addr: u16) -> u8;
    fn ppu_write(&mut self, addr: u16, val: u8);
    fn mirroring(&self) -> Mirroring;
    /// PRG RAM at $6000-$7FFF; None leaves the bus undriven (open bus).
    fn prg_ram_read(&mut self, _addr: u16) -> Option<u8> {
        None
    }
    /// Level of the cartridge's IRQ output.
    fn irq(&self) -> bool {
        false
    }
    /// Called once per CPU cycle; clocks cartridge IRQ counters and
    /// expansion audio.
    fn cpu_clock(&mut self) {}
    /// Instantaneous expansion-audio output, pre-scaled to APU units
    /// (the APU's own full scale is ~1.0).
    fn audio_sample(&self) -> f32 {
        0.0
    }
}
