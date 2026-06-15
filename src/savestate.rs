//! Savestate format: a full, deterministic snapshot of the running machine
//! (CPU, PPU, APU, controller, work RAM, and the cartridge/mapper state).
//!
//! The snapshot is serialized with `serde_json`. It deliberately omits host
//! concerns - the PPU framebuffer (regenerated on the next frame) and the
//! APU's resampling/filter chain (tied to the host output rate, reapplied by
//! the caller) - so a state restored on a machine with a different audio
//! configuration keeps playing cleanly. A state is only valid for the same
//! ROM it was taken from; [`Nes::load_state`](crate::nes::Nes::load_state)
//! checks the magic, version, and TV region before applying it.

use serde::{Deserialize, Serialize};

use crate::apu::ApuSave;
use crate::bus::BusSave;
use crate::cartridge::Region;
use crate::controller::Controller;
use crate::cpu::CpuSave;
use crate::ppu::Ppu;

/// File magic: "NSS\0" (Nes Save State), little-endian.
pub const MAGIC: u32 = 0x0053_5353;
/// Bump whenever the serialized layout changes incompatibly.
pub const VERSION: u32 = 1;

/// One complete machine snapshot.
#[derive(Serialize, Deserialize)]
pub struct SaveState {
    pub magic: u32,
    pub version: u32,
    pub region: Region,
    pub cpu: CpuSave,
    pub bus: BusSave,
    pub ppu: Ppu,
    pub apu: ApuSave,
    pub controller: Controller,
    /// Mapper-specific state, produced by [`crate::mapper::Mapper::save_state`].
    pub mapper: Vec<u8>,
}

/// `serde` helper for fixed `[u8; N]` arrays larger than 32 elements, which
/// `serde` does not derive for out of the box. Serialized as a byte sequence.
pub mod byte_array {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, const N: usize>(arr: &[u8; N], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        arr.as_slice().serialize(serializer)
    }

    pub fn deserialize<'de, D, const N: usize>(deserializer: D) -> Result<[u8; N], D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = Vec::<u8>::deserialize(deserializer)?;
        let len = v.len();
        v.try_into()
            .map_err(|_| serde::de::Error::invalid_length(len, &"a fixed-length byte array"))
    }
}
