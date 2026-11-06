//! Savestate format: a full, deterministic snapshot of the running machine
//! (CPU, PPU, APU, controller, work RAM, and the cartridge/mapper state).
//!
//! The snapshot is serialized with `serde_json`. It deliberately omits host
//! concerns - the PPU framebuffer (regenerated on the next frame) and the
//! APU's resampling/filter chain (tied to the host output rate, reapplied by
//! the caller) - and the PRG ROM, which comes from the loaded cartridge
//! rather than the state (see [`crate::impl_mapper_savestate!`]), so a state
//! restored on a machine with a different audio configuration keeps playing
//! cleanly and stays a few hundred KB at most. A state is only valid for the
//! same ROM it was taken from; [`Nes::load_state`](crate::nes::Nes::load_state)
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
/// v2 embeds the mapper state as JSON instead of a JSON array of its bytes.
pub const VERSION: u32 = 2;
/// Oldest version [`crate::nes::Nes::load_state`] still accepts.
pub const MIN_VERSION: u32 = 1;

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
    /// Port 2 pad. Optional so states written before it was captured load.
    #[serde(default)]
    pub controller2: Option<Controller>,
    /// Mapper-specific state, produced by [`crate::mapper::Mapper::save_state`].
    /// Embedded as JSON (see [`mapper_json`]); v1 files stored it as an
    /// array of byte values, three to four times larger, still accepted.
    #[serde(with = "mapper_json")]
    pub mapper: Vec<u8>,
}

/// `serde` helper embedding the mapper's JSON state as a JSON value rather
/// than as an array of its bytes. Deserializing accepts both forms.
pub mod mapper_json {
    use serde::de::Error as _;
    use serde::ser::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        if bytes.is_empty() {
            return Value::Null.serialize(serializer);
        }
        let v: Value = serde_json::from_slice(bytes).map_err(S::Error::custom)?;
        v.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::Null => Ok(Vec::new()),
            // v1: the mapper JSON's bytes as an array of numbers.
            Value::Array(items) => items
                .iter()
                .map(|b| {
                    b.as_u64()
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or_else(|| D::Error::custom("bad v1 mapper byte"))
                })
                .collect(),
            v => serde_json::to_vec(&v).map_err(D::Error::custom),
        }
    }
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
