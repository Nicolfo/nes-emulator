mod action53;
mod axrom;
mod bandai74161;
mod bandai_fcg;
mod bnrom;
mod cnrom;
mod codemasters;
mod colordreams;
mod fme7;
mod gxrom;
mod h3001;
mod holydiver;
mod irem_g101;
mod jaleco_jf17;
mod mmc1;
mod mmc2;
mod mmc3;
mod mmc4;
mod mmc5;
mod n163;
mod namco108;
mod namco175_340;
mod nrom;
mod rambo1;
mod sunsoft1;
mod sunsoft3;
mod sunsoft4;
mod taito_tc0690;
mod txsrom;
mod unrom180;
mod uxrom;
mod vrc1;
mod vrc3;
mod vrc4;
mod vrc6;
mod vrc7;

pub use action53::Action53;
pub use axrom::Axrom;
pub use bandai_fcg::BandaiFcg;
pub use bandai74161::Bandai74161;
pub use bnrom::Bnrom;
pub use cnrom::Cnrom;
pub use codemasters::Codemasters;
pub use colordreams::ColorDreams;
pub use fme7::Fme7;
pub use gxrom::Gxrom;
pub use h3001::H3001;
pub use holydiver::HolyDiver;
pub use irem_g101::IremG101;
pub use jaleco_jf17::JalecoJf17;
pub use mmc1::Mmc1;
pub use mmc2::Mmc2;
pub use mmc3::Mmc3;
pub use mmc4::Mmc4;
pub use mmc5::Mmc5;
pub use n163::N163;
pub use namco108::Namco108;
pub use namco175_340::Namco175340;
pub use nrom::Nrom;
pub use rambo1::Rambo1;
pub use sunsoft1::Sunsoft1;
pub use sunsoft3::Sunsoft3;
pub use sunsoft4::Sunsoft4;
pub use taito_tc0690::TaitoTc0690;
pub use txsrom::Txsrom;
pub use unrom180::Unrom180;
pub use uxrom::Uxrom;
pub use vrc1::Vrc1;
pub use vrc3::Vrc3;
pub use vrc4::Vrc4;
pub use vrc6::Vrc6;
pub use vrc7::Vrc7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mirroring {
    Horizontal,
    Vertical,
    SingleScreenLo,
    SingleScreenHi,
    /// All four nametables are distinct. The board carries an extra 2KB of
    /// RAM (alongside the console's 2KB CIRAM) wired via the cartridge's
    /// four-screen pad, so the mapper's own mirroring control is bypassed.
    FourScreen,
}

/// Where a nametable access ($2000-$3EFF) is routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NtTarget {
    /// Offset into the console's 2KB CIRAM.
    Ciram(u16),
    /// The cartridge serves/accepts the byte via ppu_read/ppu_write.
    Cart,
}

/// Nametable RAM offset for a nametable address. For the plain mirrorings
/// this is an index into the console's 2KB CIRAM; under four-screen it spans
/// the full 4KB (CIRAM plus the cartridge's extra 2KB), addressed linearly.
pub fn mirror_nt(mirroring: Mirroring, addr: u16) -> u16 {
    let a = addr & 0x0FFF;
    match mirroring {
        Mirroring::Vertical => a & 0x07FF,
        Mirroring::Horizontal => ((a >> 1) & 0x400) | (a & 0x3FF),
        Mirroring::SingleScreenLo => a & 0x3FF,
        Mirroring::SingleScreenHi => 0x400 | (a & 0x3FF),
        Mirroring::FourScreen => a,
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
    /// Raw PRG RAM contents for battery (.sav) persistence; None when the
    /// board has no PRG RAM. Bypasses any RAM-enable/banking logic.
    fn prg_ram(&self) -> Option<&[u8]> {
        None
    }
    /// Mutable PRG RAM view for restoring a .sav file at power-on.
    fn prg_ram_mut(&mut self) -> Option<&mut [u8]> {
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
    /// Observe CPU writes to the PPU registers ($2000-$3FFF); the MMC5
    /// snoops $2000/$2001 for sprite size and rendering state.
    fn cpu_bus_write(&mut self, _addr: u16, _val: u8) {}

    /// Serialize the mapper's full state (banking registers, IRQ counters,
    /// PRG/CHR RAM, expansion audio) for a savestate. Implemented per mapper
    /// via [`impl_mapper_savestate!`]; the bytes are opaque JSON tied to the
    /// concrete mapper type and only valid for the same ROM.
    fn save_state(&self) -> Vec<u8>;

    /// Restore state previously produced by [`Mapper::save_state`]. Returns an
    /// error if the bytes don't match this mapper (e.g. a state from a
    /// different ROM).
    fn load_state(&mut self, data: &[u8]) -> Result<(), String>;
}

/// Implements [`Mapper::save_state`]/[`Mapper::load_state`] for a mapper type
/// by round-tripping the struct through `serde_json`. The mapper must derive
/// `Serialize`/`Deserialize`, have a `prg: Vec<u8>` and a `chr: Vec<u8>` field,
/// and mark `prg` with `#[serde(skip)]`. Invoke inside the `impl Mapper` block.
///
/// PRG ROM is never embedded in the blob: it is elided on save (via the
/// `#[serde(skip)]` on the field) and re-injected from the live cartridge on
/// restore. This keeps savestates small and makes a malformed blob unable to
/// shrink `prg` into an out-of-bounds bank index (`len()/bank - 1`, `% banks`).
/// CHR length is checked against the loaded board on restore for the same
/// reason.
///
/// Two forms:
/// - `impl_mapper_savestate!()` keeps `chr` in the blob (correct whether the
///   board's CHR is ROM or RAM).
/// - `impl_mapper_savestate!(chr_is_ram = field)` additionally elides CHR-ROM
///   (re-injected on restore) while keeping CHR-RAM, which is genuine state.
///   Requires the mapper to also derive `Clone`.
#[macro_export]
macro_rules! impl_mapper_savestate {
    () => {
        fn save_state(&self) -> ::std::vec::Vec<u8> {
            // `prg` carries `#[serde(skip)]`, so the ROM is not embedded here.
            ::serde_json::to_vec(self).expect("serialize mapper state")
        }

        fn load_state(&mut self, data: &[u8]) -> ::std::result::Result<(), ::std::string::String> {
            let mut restored: Self = ::serde_json::from_slice(data).map_err(|e| e.to_string())?;
            if restored.chr.len() != self.chr.len() {
                return ::std::result::Result::Err(::std::format!(
                    "savestate CHR size {} does not match ROM ({})",
                    restored.chr.len(),
                    self.chr.len()
                ));
            }
            restored.prg = ::std::mem::take(&mut self.prg);
            *self = restored;
            ::std::result::Result::Ok(())
        }
    };

    (chr_is_ram = $flag:ident) => {
        fn save_state(&self) -> ::std::vec::Vec<u8> {
            if self.$flag {
                // CHR is RAM: its contents are state, so keep them in the blob.
                ::serde_json::to_vec(self).expect("serialize mapper state")
            } else {
                // CHR is ROM: blank it in a copy so it is elided like PRG.
                let mut shadow = ::std::clone::Clone::clone(self);
                shadow.chr = ::std::vec::Vec::new();
                ::serde_json::to_vec(&shadow).expect("serialize mapper state")
            }
        }

        fn load_state(&mut self, data: &[u8]) -> ::std::result::Result<(), ::std::string::String> {
            let mut restored: Self = ::serde_json::from_slice(data).map_err(|e| e.to_string())?;
            if restored.$flag {
                if restored.chr.len() != self.chr.len() {
                    return ::std::result::Result::Err(::std::format!(
                        "savestate CHR-RAM size {} does not match board ({})",
                        restored.chr.len(),
                        self.chr.len()
                    ));
                }
            } else {
                // CHR-ROM was elided: re-inject it from the live cartridge.
                restored.chr = ::std::mem::take(&mut self.chr);
            }
            restored.prg = ::std::mem::take(&mut self.prg);
            *self = restored;
            ::std::result::Result::Ok(())
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_screen_keeps_all_nametables_distinct() {
        // The four nametables at $2000/$2400/$2800/$2C00 must map to four
        // separate 1KB regions of the 4KB nametable RAM.
        let offs: Vec<u16> = [0x2000, 0x2400, 0x2800, 0x2C00]
            .iter()
            .map(|&a| mirror_nt(Mirroring::FourScreen, a))
            .collect();
        assert_eq!(offs, vec![0x000, 0x400, 0x800, 0xC00]);
        // Top of the range stays within the 4KB window.
        assert_eq!(mirror_nt(Mirroring::FourScreen, 0x2FFF), 0xFFF);
    }
}
