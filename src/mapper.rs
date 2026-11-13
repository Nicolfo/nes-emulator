mod action53;
mod axrom;
mod bandai74161;
mod bandai_fcg;
mod bnrom;
mod chr;
mod cnrom;
mod cnrom_protect;
mod codemasters;
mod colordreams;
mod discrete;
mod fds;
mod fds_audio;
mod flash;
mod fme7;
mod fong_shen_bang;
mod gtrom;
mod gxrom;
mod h3001;
mod holydiver;
mod irem_discrete;
mod irem_g101;
mod jaleco_jf17;
mod jaleco_latch;
mod jaleco_ss88006;
mod mmc1;
mod mmc2;
mod mmc3;
mod mmc4;
mod mmc5;
mod multicart;
mod n163;
mod namco108;
mod namco175_340;
mod nrom;
mod nsf;
mod rambo1;
mod sunsoft1;
mod sunsoft2;
mod sunsoft3;
mod sunsoft4;
mod taito_tc0690;
mod taito_x1;
mod txsrom;
mod unrom180;
mod unrom512;
mod uxrom;
mod vrc1;
mod vrc3;
mod vrc4;
mod vrc6;
mod vrc7;
mod vs_unisystem;

pub use action53::Action53;
pub use axrom::Axrom;
pub use bandai_fcg::BandaiFcg;
pub use bandai74161::Bandai74161;
pub use bnrom::Bnrom;
pub use chr::{Chr, StateField};
pub use cnrom::Cnrom;
pub use cnrom_protect::CnromProtect;
pub use codemasters::Codemasters;
pub use colordreams::ColorDreams;
pub use discrete::{Latch, LatchKind};
pub use fds::{Fds, SIDE_SIZE as FDS_SIDE_SIZE, parse_disk_image as parse_fds_image};
pub use fme7::Fme7;
pub use fong_shen_bang::FongShenBang;
pub use gtrom::Gtrom;
pub use gxrom::Gxrom;
pub use h3001::H3001;
pub use holydiver::HolyDiver;
pub use irem_discrete::{IremLrog017, IremTamS1};
pub use irem_g101::IremG101;
pub use jaleco_jf17::JalecoJf17;
pub use jaleco_latch::JalecoLatch;
pub use jaleco_ss88006::JalecoSs88006;
pub use mmc1::Mmc1;
pub use mmc2::Mmc2;
pub use mmc3::{Mmc3, Mmc3Board};
pub use mmc4::Mmc4;
pub use mmc5::Mmc5;
pub use multicart::{Action52, Caltron, K1029, Quattro, RumbleStation};
pub use n163::N163;
pub use namco108::{Namco108, Namco108Board};
pub use namco175_340::Namco175340;
pub use nrom::Nrom;
pub use nsf::NsfMapper;
pub use rambo1::Rambo1;
pub use sunsoft1::Sunsoft1;
pub use sunsoft2::Sunsoft2;
pub use sunsoft3::Sunsoft3;
pub use sunsoft4::Sunsoft4;
pub use taito_tc0690::TaitoTc0690;
pub use taito_x1::{TaitoX1005, TaitoX1017};
pub use txsrom::Txsrom;
pub use unrom180::Unrom180;
pub use unrom512::Unrom512;
pub use uxrom::Uxrom;
pub use vrc1::Vrc1;
pub use vrc3::Vrc3;
pub use vrc4::Vrc4;
pub use vrc6::Vrc6;
pub use vrc7::Vrc7;
pub use vs_unisystem::VsUnisystem;

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
    /// Resize the board's work RAM to the NES 2.0 header-declared sizes
    /// (bytes, already rounded up to whole 8KB by the loader; 0 keeps the
    /// board default). `chr_ram` applies only when the board has CHR RAM
    /// rather than CHR ROM. Called once by `load_rom` right after
    /// construction, before the trainer or a .sav restore touches the RAM.
    /// The default ignores the sizes - right for boards with no RAM at all.
    fn set_ram_sizes(&mut self, _prg_ram: usize, _chr_ram: usize) {}
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
    /// Vs. System: observe $4016 writes (the controller strobe register),
    /// whose bit 2 drives the Vs. UniSystem board's bank select.
    fn vs_io_write(&mut self, _val: u8) {}
    /// Vs. DualSystem: whether this board owns the shared work RAM.
    fn vs_ram_access(&self) -> bool {
        true
    }
    /// Vs. DualSystem: give or take this board's access to the shared RAM.
    fn vs_set_ram_access(&mut self, _on: bool) {}

    /// Bytes persisted to the `.sav` file (when the header's battery bit is
    /// set). Defaults to the PRG RAM; flash-saving boards return their PRG
    /// flash instead.
    fn battery_save(&self) -> Option<&[u8]> {
        self.prg_ram()
    }
    /// Restore a `.sav` blob produced by [`Mapper::battery_save`]. The
    /// default copies the overlapping prefix into PRG RAM and calls
    /// [`Mapper::battery_ram_loaded`]; flash boards accept only an exact-size
    /// image (anything else would overwrite the program).
    fn battery_restore(&mut self, data: &[u8]) {
        if let Some(ram) = self.prg_ram_mut() {
            let n = ram.len().min(data.len());
            ram[..n].copy_from_slice(&data[..n]);
        }
        self.battery_ram_loaded();
    }

    /// Called after a .sav blob was copied into [`Mapper::prg_ram_mut`], for
    /// boards whose persisted bytes feed derived state (the FDS rebuilds its
    /// drive's gapped disk streams from the restored `.fds` image).
    fn battery_ram_loaded(&mut self) {}

    /// Famicom Disk System: the BIOS is checking for the disk whose 10-byte
    /// ID is `expected` ($FF = any); boards may insert the matching side.
    fn fds_bios_disk_check(&mut self, _expected: &[u8; 10]) {}
    /// Famicom Disk System: enable/disable automatic side insertion.
    fn fds_set_auto_insert(&mut self, _on: bool) {}
    /// Famicom Disk System: number of disk sides; None for cartridges.
    fn fds_side_count(&self) -> Option<usize> {
        None
    }
    /// Famicom Disk System: side currently in the drive (None = ejected,
    /// mid-swap, or not an FDS).
    fn fds_current_side(&self) -> Option<usize> {
        None
    }
    /// Famicom Disk System: eject (None) or swap in a side. Insertion lands
    /// after a short empty-drive delay so the BIOS notices the change.
    fn fds_insert(&mut self, _side: Option<usize>) {}

    /// NSF metadata when this "cartridge" is an NSF music rip; None for
    /// real boards.
    fn nsf_info(&self) -> Option<crate::nsf::NsfInfo> {
        None
    }
    /// Select NSF song `song` (0-based, clamped) and rewind the board to its
    /// power-on banking so a following CPU reset re-runs the driver into
    /// that track. No-op for real boards.
    fn nsf_select(&mut self, _song: usize) {}

    /// Whether the board carries a speech chip whose recordings the host
    /// must supply (see [`Mapper::set_speech_samples`]).
    fn has_speech(&self) -> bool {
        false
    }
    /// Supply the speech chip's recordings, which live in an undumped mask
    /// ROM: mono clips in -1..1 at `_sample_rate`, indexed by the chip's
    /// sample number (`None` = not available). No-op for other boards.
    fn set_speech_samples(&mut self, _samples: Vec<Option<Vec<f32>>>, _sample_rate: u32) {}

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
/// by round-tripping the whole struct through `serde_json`. The mapper must
/// derive `Serialize`/`Deserialize` and mark its `prg` ROM `#[serde(skip)]`:
/// the ROM bytes come from the loaded cartridge, not the state, which keeps
/// savestates a few hundred KB at most instead of embedding up to a megabyte
/// of PRG ROM. `load_state` grafts the live `prg` onto the restored state.
/// Invoke inside the `impl Mapper` block.
///
/// Listing field names (`impl_mapper_savestate!(chr, prg_ram)`) restores
/// each through [`StateField::restore`]: `load_state` rejects a state whose
/// named buffers differ in length from the live instance's (`Vec`-sized RAM
/// has no inherent length check on deserialize, unlike a fixed array, so
/// without the guard a crafted state could shrink a buffer and panic later
/// bank math; a same-ROM state always matches because the sizes are derived
/// from the ROM header). CHR is held in a [`Chr`], which serializes as
/// `null` until written - CHR ROM never enters the state - and gets its
/// power-on image grafted back from the live instance here. A listed `Chr`
/// field is therefore required, not just checked.
#[macro_export]
macro_rules! impl_mapper_savestate {
    ($($field:ident),* $(,)?) => {
        fn save_state(&self) -> ::std::vec::Vec<u8> {
            ::serde_json::to_vec(self).expect("serialize mapper state")
        }

        fn load_state(&mut self, data: &[u8]) -> ::std::result::Result<(), ::std::string::String> {
            let mut new: Self = ::serde_json::from_slice(data).map_err(|e| e.to_string())?;
            $(
                $crate::mapper::StateField::restore(
                    &mut new.$field,
                    &mut self.$field,
                    stringify!($field),
                )?;
            )*
            new.prg = ::std::mem::take(&mut self.prg);
            *self = new;
            Ok(())
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
