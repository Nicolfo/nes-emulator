//! CHR memory that keeps read-only data out of savestates.
//!
//! A board's CHR is either ROM (from the cartridge, never written) or RAM.
//! [`Chr`] doesn't need to be told which: it is copy-on-write over the
//! power-on image and remembers whether it was ever mutated. An untouched
//! `Chr` serializes as `null` - CHR ROM (and CHR RAM nobody wrote yet) costs
//! nothing in a savestate, and restoring it grafts the live instance's
//! pristine image back in (see [`StateField`]). Once anything takes a
//! mutable borrow (`chr[i] = v`, `resize`, ...), the data serializes in
//! full exactly as a plain `Vec<u8>` did, so CHR RAM is preserved and states
//! written before this type existed (a full array) still load.
//!
//! Reads go through `Deref<Target = Vec<u8>>`, so indexing, `len()` and
//! slicing work unchanged; only genuine mutable access marks it dirty.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Debug, Default)]
pub struct Chr {
    /// Current contents; shares `base`'s buffer until first written.
    data: Arc<Vec<u8>>,
    /// Power-on image (CHR ROM, or CHR RAM's initial contents).
    base: Arc<Vec<u8>>,
    /// Mutably borrowed since power-on (or restored from a state whose data
    /// differs from `base`).
    dirty: bool,
}

impl Chr {
    pub fn new(data: Vec<u8>) -> Self {
        let base = Arc::new(data);
        Chr {
            data: base.clone(),
            base,
            dirty: false,
        }
    }

    /// Whether the contents can differ from the power-on image.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

impl From<Vec<u8>> for Chr {
    fn from(v: Vec<u8>) -> Self {
        Chr::new(v)
    }
}

impl Deref for Chr {
    type Target = Vec<u8>;
    fn deref(&self) -> &Vec<u8> {
        &self.data
    }
}

impl DerefMut for Chr {
    fn deref_mut(&mut self) -> &mut Vec<u8> {
        self.dirty = true;
        Arc::make_mut(&mut self.data)
    }
}

impl Serialize for Chr {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.dirty {
            self.data.as_slice().serialize(s)
        } else {
            s.serialize_none()
        }
    }
}

impl<'de> Deserialize<'de> for Chr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // `null` = unchanged from power-on; the image is grafted in by
        // `StateField::restore`. An array is real data (CHR RAM, or a state
        // from before CHR ROM was left out).
        Ok(match Option::<Vec<u8>>::deserialize(d)? {
            Some(v) => Chr {
                data: Arc::new(v),
                base: Arc::new(Vec::new()),
                dirty: true,
            },
            None => Chr::default(),
        })
    }
}

/// A savestate field restored against the live instance in
/// [`crate::impl_mapper_savestate!`]: buffer sizes must match the ROM
/// (rejecting a state from a different game), and [`Chr`] takes its
/// power-on image from the live instance.
pub trait StateField {
    fn restore(&mut self, live: &mut Self, name: &str) -> Result<(), String>;
}

fn size_mismatch(name: &str, state: usize, live: usize) -> String {
    format!("savestate {name} is {state} bytes; this ROM uses {live}")
}

impl<T> StateField for Vec<T> {
    fn restore(&mut self, live: &mut Self, name: &str) -> Result<(), String> {
        if self.len() != live.len() {
            return Err(size_mismatch(name, self.len(), live.len()));
        }
        Ok(())
    }
}

impl StateField for Chr {
    fn restore(&mut self, live: &mut Self, name: &str) -> Result<(), String> {
        if !self.dirty {
            // Untouched at save time: exactly the power-on image.
            self.base = live.base.clone();
            self.data = live.base.clone();
            return Ok(());
        }
        if self.data.len() != live.len() {
            return Err(size_mismatch(name, self.data.len(), live.len()));
        }
        self.base = live.base.clone();
        if *self.data == *self.base {
            // An old-format state carrying unchanged CHR ROM: share the
            // ROM buffer again so later states stay small.
            self.data = self.base.clone();
            self.dirty = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untouched_chr_serializes_as_null_and_restores_from_live() {
        let mut live = Chr::new(vec![7; 0x2000]);
        let json = serde_json::to_string(&live).unwrap();
        assert_eq!(json, "null");
        let mut restored: Chr = serde_json::from_str(&json).unwrap();
        restored.restore(&mut live, "chr").unwrap();
        assert_eq!(*restored, vec![7; 0x2000]);
        assert!(!restored.is_dirty());
    }

    #[test]
    fn written_chr_serializes_in_full() {
        let mut ram = Chr::new(vec![0; 16]);
        let _ = ram[3]; // a read doesn't dirty it
        assert!(!ram.is_dirty());
        ram[3] = 9;
        assert!(ram.is_dirty());
        let json = serde_json::to_string(&ram).unwrap();
        let mut live = Chr::new(vec![0; 16]);
        let mut restored: Chr = serde_json::from_str(&json).unwrap();
        restored.restore(&mut live, "chr").unwrap();
        assert_eq!(restored[3], 9);
        // Copy-on-write: the live instance's image is untouched.
        assert_eq!(live[3], 0);
    }

    #[test]
    fn old_full_array_state_loads_and_goes_clean_when_unchanged() {
        let mut live = Chr::new(vec![1, 2, 3, 4]);
        let mut restored: Chr = serde_json::from_str("[1,2,3,4]").unwrap();
        restored.restore(&mut live, "chr").unwrap();
        assert!(!restored.is_dirty());
        assert_eq!(serde_json::to_string(&restored).unwrap(), "null");
        let mut wrong: Chr = serde_json::from_str("[1,2,3]").unwrap();
        assert!(wrong.restore(&mut live, "chr").is_err());
    }
}
