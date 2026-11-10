//! SST39SF010/020/040 parallel NOR flash, as used for self-saving PRG on
//! homebrew boards (UNROM 512, GTROM).
//!
//! The chip owns the PRG image. Commands are the JEDEC unlock sequence
//! `$AA -> $5555, $55 -> $2AAA` followed by:
//! - `$A0 -> $5555`, then one write programs a byte (bits can only be
//!   cleared, so the stored value is ANDed with the written one);
//! - `$80 -> $5555`, a second unlock, then `$30 -> sector` (4KB sector
//!   erase) or `$10 -> $5555` (chip erase), filling with `$FF`;
//! - `$90 -> $5555` enters software-ID mode (reads return the manufacturer
//!   `$BF` at even addresses and the device ID at odd ones) until `$F0`
//!   (alone or after an unlock) returns to read mode.
//!
//! Command addresses compare the low 15 bits of the chip address, as the
//! real part does. Program and erase complete instantly: a game polling
//! DQ7/DQ6 for completion reads the final data straight away.
//!
//! Savestates: the image itself comes from the cartridge (like every
//! mapper's PRG ROM), so a state stores only the 4KB sectors changed since
//! power-on (or since the last `.sav` restore). States of boards that
//! never write stay as small as before.

use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Erase granularity.
pub(crate) const SECTOR: usize = 0x1000;

/// Command sequence position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
enum Step {
    #[default]
    Idle,
    /// Got `$AA` at `$5555`.
    Unlock1,
    /// Got `$55` at `$2AAA`: the next write is the command.
    Unlock2,
    /// `$A0` command: the next write programs a byte.
    Program,
}

#[derive(Default)]
pub(crate) struct Sst39sf {
    /// Current flash contents (the PRG image).
    image: Vec<u8>,
    /// Contents at power-on / after the last `.sav` restore; sectors are
    /// reverted to this before a savestate's changed sectors are applied.
    base: Vec<u8>,
    /// Sectors that differ from `base` (indices of 4KB sectors).
    dirty: BTreeSet<u32>,
    step: Step,
    /// Got `$AA $55 $80`: an erase command follows a second unlock.
    erase_armed: bool,
    id_mode: bool,
    /// Changed sectors carried by a deserialized savestate, applied by
    /// [`Sst39sf::graft`].
    pending: Vec<(u32, Vec<u8>)>,
}

/// Serialized form: command state plus the changed sectors' contents.
#[derive(Serialize, Deserialize)]
struct FlashSave {
    step: Step,
    erase_armed: bool,
    id_mode: bool,
    #[serde(default)]
    sectors: Vec<(u32, Vec<u8>)>,
}

impl Serialize for Sst39sf {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let sectors = self
            .dirty
            .iter()
            .filter_map(|&i| {
                let start = i as usize * SECTOR;
                let end = (start + SECTOR).min(self.image.len());
                (start < end).then(|| (i, self.image[start..end].to_vec()))
            })
            .collect();
        FlashSave {
            step: self.step,
            erase_armed: self.erase_armed,
            id_mode: self.id_mode,
            sectors,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Sst39sf {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let f = FlashSave::deserialize(d)?;
        Ok(Sst39sf {
            step: f.step,
            erase_armed: f.erase_armed,
            id_mode: f.id_mode,
            pending: f.sectors,
            ..Sst39sf::default()
        })
    }
}

impl Sst39sf {
    pub(crate) fn new(image: Vec<u8>) -> Self {
        Sst39sf {
            base: image.clone(),
            image,
            ..Sst39sf::default()
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.image.len()
    }

    /// The whole chip, as persisted to `.sav`.
    pub(crate) fn image(&self) -> &[u8] {
        &self.image
    }

    /// Whether any sector differs from the power-on image.
    #[cfg(test)]
    pub(crate) fn modified(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Device ID by capacity: SST39SF010 / 020 / 040.
    fn device_id(&self) -> u8 {
        match self.image.len() {
            0..=0x20000 => 0xB5,
            0x20001..=0x40000 => 0xB6,
            _ => 0xB7,
        }
    }

    /// Read chip byte `index` (already wrapped into the image by the board).
    pub(crate) fn read(&self, index: usize) -> u8 {
        if self.id_mode {
            return if index & 1 == 0 {
                0xBF
            } else {
                self.device_id()
            };
        }
        self.image[index % self.image.len().max(1)]
    }

    /// CPU write reaching the chip at byte `index`.
    pub(crate) fn write(&mut self, index: usize, val: u8) {
        if self.image.is_empty() {
            return;
        }
        let cmd_addr = index & 0x7FFF;
        if self.step == Step::Program {
            let i = index % self.image.len();
            self.image[i] &= val;
            self.mark(i);
            self.step = Step::Idle;
            return;
        }
        if val == 0xF0 {
            // Reset / software-ID exit, from any point in a sequence.
            self.step = Step::Idle;
            self.erase_armed = false;
            self.id_mode = false;
            return;
        }
        self.step = match (self.step, cmd_addr, val) {
            (Step::Idle, 0x5555, 0xAA) => Step::Unlock1,
            (Step::Unlock1, 0x2AAA, 0x55) => Step::Unlock2,
            (Step::Unlock2, _, _) => {
                self.command(index, cmd_addr, val);
                return;
            }
            _ => {
                self.erase_armed = false;
                Step::Idle
            }
        };
    }

    fn command(&mut self, index: usize, cmd_addr: usize, val: u8) {
        self.step = Step::Idle;
        let armed = std::mem::take(&mut self.erase_armed);
        match (armed, cmd_addr, val) {
            (false, 0x5555, 0xA0) => self.step = Step::Program,
            (false, 0x5555, 0x80) => self.erase_armed = true,
            (false, 0x5555, 0x90) => self.id_mode = true,
            (true, _, 0x30) => {
                let start = (index % self.image.len()) & !(SECTOR - 1);
                let end = (start + SECTOR).min(self.image.len());
                self.image[start..end].fill(0xFF);
                self.mark(start);
            }
            (true, 0x5555, 0x10) => {
                self.image.fill(0xFF);
                for i in (0..self.image.len()).step_by(SECTOR) {
                    self.mark(i);
                }
            }
            _ => {}
        }
    }

    fn mark(&mut self, index: usize) {
        self.dirty.insert((index / SECTOR) as u32);
    }

    /// Replace the chip contents with a `.sav` image. Returns false (and
    /// changes nothing) when the size doesn't match the chip, e.g. a save
    /// from a different build of the game.
    pub(crate) fn restore(&mut self, data: &[u8]) -> bool {
        if data.len() != self.image.len() || data.is_empty() {
            return false;
        }
        self.image.copy_from_slice(data);
        self.base = self.image.clone();
        self.dirty.clear();
        self.step = Step::Idle;
        self.erase_armed = false;
        self.id_mode = false;
        true
    }

    /// Finish restoring a deserialized savestate: take the image and base
    /// from the live chip, revert to the base and apply the state's
    /// changed sectors. Errors on sectors that don't fit this chip.
    pub(crate) fn graft(&mut self, live: &mut Sst39sf) -> Result<(), String> {
        let base = std::mem::take(&mut live.base);
        let mut image = base.clone();
        let mut dirty = BTreeSet::new();
        for (i, data) in std::mem::take(&mut self.pending) {
            let start = i as usize * SECTOR;
            let end = (start + SECTOR).min(image.len());
            if start >= end || data.len() != end - start {
                live.base = base;
                return Err(format!("savestate flash sector {i} doesn't fit this ROM"));
            }
            image[start..end].copy_from_slice(&data);
            dirty.insert(i);
        }
        self.image = image;
        self.base = base;
        self.dirty = dirty;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unlock(f: &mut Sst39sf) {
        f.write(0x5555, 0xAA);
        f.write(0x2AAA, 0x55);
    }

    fn chip() -> Sst39sf {
        Sst39sf::new((0..0x80000).map(|i| (i >> 12) as u8).collect())
    }

    #[test]
    fn byte_program_clears_bits_only() {
        let mut f = chip();
        unlock(&mut f);
        f.write(0x5555, 0xA0);
        f.write(0x3_1234, 0x0F); // sector $31 holds $31
        assert_eq!(f.read(0x3_1234), 0x31 & 0x0F);
        // A plain write without the sequence does nothing.
        f.write(0x3_1235, 0x00);
        assert_eq!(f.read(0x3_1235), 0x31);
        assert!(f.modified());
    }

    #[test]
    fn sector_and_chip_erase() {
        let mut f = chip();
        unlock(&mut f);
        f.write(0x5555, 0x80);
        unlock(&mut f);
        f.write(0x2_3456, 0x30);
        assert_eq!(f.read(0x2_3000), 0xFF);
        assert_eq!(f.read(0x2_3FFF), 0xFF);
        assert_eq!(f.read(0x2_4000), 0x24, "next sector untouched");
        assert_eq!(f.read(0x2_2FFF), 0x22, "previous sector untouched");

        unlock(&mut f);
        f.write(0x5555, 0x80);
        unlock(&mut f);
        f.write(0x5555, 0x10);
        assert!(f.image().iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn software_id_entry_and_exit() {
        let mut f = chip();
        unlock(&mut f);
        f.write(0x5555, 0x90);
        assert_eq!(f.read(0), 0xBF);
        assert_eq!(f.read(1), 0xB7, "512KB part is an SST39SF040");
        f.write(0x0000, 0xF0);
        assert_eq!(f.read(1), 0x00);
        // Exit via the unlocked form too.
        unlock(&mut f);
        f.write(0x5555, 0x90);
        unlock(&mut f);
        f.write(0x5555, 0xF0);
        assert_eq!(f.read(0x1000), 0x01);
        let small = Sst39sf::new(vec![0; 0x20000]);
        assert_eq!(small.device_id(), 0xB5);
    }

    #[test]
    fn broken_sequence_resets() {
        let mut f = chip();
        f.write(0x5555, 0xAA);
        f.write(0x1234, 0x55); // wrong address
        f.write(0x5555, 0xA0);
        f.write(0x0000, 0x00);
        assert_eq!(f.read(0), 0x00, "already zero");
        f.write(0x4000, 0x00);
        assert_eq!(f.read(0x4000), 0x04, "no program happened");
        assert!(!f.modified());
    }

    #[test]
    fn savestate_keeps_only_changed_sectors() {
        let mut f = chip();
        let clean = serde_json::to_vec(&f).unwrap();
        unlock(&mut f);
        f.write(0x5555, 0xA0);
        f.write(0x7_0000, 0x00);
        let json = serde_json::to_vec(&f).unwrap();
        assert!(
            json.len() < clean.len() + 5 * SECTOR,
            "one sector, not the chip"
        );

        // Diverge after the snapshot, then restore it onto the live chip.
        unlock(&mut f);
        f.write(0x5555, 0xA0);
        f.write(0x1_0000, 0x00);
        let mut restored: Sst39sf = serde_json::from_slice(&json).unwrap();
        restored.graft(&mut f).unwrap();
        assert_eq!(restored.read(0x7_0000), 0x00, "state's write kept");
        assert_eq!(restored.read(0x1_0000), 0x10, "later write reverted");
        assert_eq!(restored.len(), 0x80000);
    }

    #[test]
    fn restore_requires_exact_size() {
        let mut f = chip();
        assert!(!f.restore(&[0u8; 100]));
        assert_eq!(f.read(0x1000), 0x01);
        assert!(f.restore(&vec![0x42; 0x80000]));
        assert_eq!(f.read(0x1000), 0x42);
        assert!(!f.modified(), ".sav contents are the new baseline");
    }

    #[test]
    fn graft_rejects_foreign_sectors() {
        let mut big = chip();
        unlock(&mut big);
        big.write(0x5555, 0xA0);
        big.write(0x7_F000, 0x00);
        let json = serde_json::to_vec(&big).unwrap();
        let mut small = Sst39sf::new(vec![0; 0x20000]);
        let mut restored: Sst39sf = serde_json::from_slice(&json).unwrap();
        assert!(restored.graft(&mut small).is_err());
        assert_eq!(small.base.len(), 0x20000, "live chip left intact");
    }
}
