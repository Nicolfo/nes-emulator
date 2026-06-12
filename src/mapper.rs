mod axrom;
mod cnrom;
mod colordreams;
mod fme7;
mod gxrom;
mod mmc1;
mod mmc2;
mod mmc3;
mod n163;
mod nrom;
mod uxrom;
mod vrc6;

pub use axrom::Axrom;
pub use cnrom::Cnrom;
pub use colordreams::ColorDreams;
pub use fme7::Fme7;
pub use gxrom::Gxrom;
pub use mmc1::Mmc1;
pub use mmc2::Mmc2;
pub use mmc3::Mmc3;
pub use n163::N163;
pub use nrom::Nrom;
pub use uxrom::Uxrom;
pub use vrc6::Vrc6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirroring {
    Horizontal,
    Vertical,
    SingleScreenLo,
    SingleScreenHi,
}

/// Where a nametable access ($2000-$3EFF) is routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NtTarget {
    /// Offset into the console's 2KB CIRAM.
    Ciram(u16),
    /// The cartridge serves/accepts the byte via ppu_read/ppu_write.
    Cart,
}

/// CIRAM offset for a nametable address under plain mirroring.
pub fn mirror_nt(mirroring: Mirroring, addr: u16) -> u16 {
    let a = addr & 0x0FFF;
    match mirroring {
        Mirroring::Vertical => a & 0x07FF,
        Mirroring::Horizontal => ((a >> 1) & 0x400) | (a & 0x3FF),
        Mirroring::SingleScreenLo => a & 0x3FF,
        Mirroring::SingleScreenHi => 0x400 | (a & 0x3FF),
    }
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
    /// Readable cartridge registers in $4020-$5FFF; None is open bus.
    fn cpu_reg_read(&mut self, _addr: u16) -> Option<u8> {
        None
    }
    /// Route a PPU nametable access. Called for every NT-range fetch, so
    /// mappers may also use it to observe the PPU bus.
    fn nt_target(&mut self, addr: u16) -> NtTarget {
        NtTarget::Ciram(mirror_nt(self.mirroring(), addr))
    }
}
