//! Rewind history: a bounded ring of machine snapshots with content-defined
//! chunk deduplication.
//!
//! Consecutive snapshots are nearly identical (CHR ROM, most of RAM and VRAM
//! don't change between frames), but they are JSON text, so a changed value
//! with a different digit count shifts everything after it and defeats a
//! fixed-offset diff. Content-defined chunking cuts each snapshot where a
//! rolling hash of the *content* hits a boundary pattern, so boundaries
//! re-synchronise right after an edit and unchanged regions produce the same
//! chunks. Each chunk is stored once (refcounted); a snapshot is just the
//! list of its chunk ids. A minute of history costs a few MB instead of
//! hundreds.

use std::collections::hash_map::{DefaultHasher, Entry};
use std::collections::{HashMap, VecDeque};
use std::hash::Hasher;

/// Chunk size bounds; the boundary mask targets ~1 KB average chunks.
const MIN_CHUNK: usize = 256;
const MAX_CHUNK: usize = 8192;
const BOUNDARY_MASK: u64 = 0x3FF << 20;

/// Gear-hash table: one pseudo-random 64-bit value per byte value.
const GEAR: [u64; 256] = {
    let mut t = [0u64; 256];
    let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut i = 0;
    while i < 256 {
        // splitmix64
        s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        t[i] = z ^ (z >> 31);
        i += 1;
    }
    t
};

/// Split `data` into content-defined chunks.
fn chunks(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = data;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut h = 0u64;
        let mut cut = rest.len().min(MAX_CHUNK);
        for (i, &b) in rest.iter().enumerate().take(MAX_CHUNK).skip(MIN_CHUNK) {
            h = (h << 1).wrapping_add(GEAR[b as usize]);
            if h & BOUNDARY_MASK == 0 {
                cut = i + 1;
                break;
            }
        }
        let (chunk, tail) = rest.split_at(cut);
        rest = tail;
        Some(chunk)
    })
}

struct Chunk {
    data: Box<[u8]>,
    refs: usize,
}

/// Bounded snapshot history (newest at the back).
pub struct Rewind {
    store: HashMap<u64, Chunk>,
    frames: VecDeque<Vec<u64>>,
    capacity: usize,
    stored_bytes: usize,
}

impl Rewind {
    /// Keep at most `capacity` snapshots; older ones are dropped.
    pub fn new(capacity: usize) -> Self {
        Rewind {
            store: HashMap::new(),
            frames: VecDeque::new(),
            capacity: capacity.max(1),
            stored_bytes: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Bytes of unique chunk data currently held.
    pub fn stored_bytes(&self) -> usize {
        self.stored_bytes
    }

    pub fn clear(&mut self) {
        self.store.clear();
        self.frames.clear();
        self.stored_bytes = 0;
    }

    /// Append a snapshot, evicting the oldest when full.
    pub fn push(&mut self, snapshot: &[u8]) {
        if self.frames.len() == self.capacity
            && let Some(old) = self.frames.pop_front()
        {
            self.release(&old);
        }
        let ids = chunks(snapshot).map(|c| self.intern(c)).collect();
        self.frames.push_back(ids);
    }

    /// Remove and return the newest snapshot.
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        let ids = self.frames.pop_back()?;
        let mut out = Vec::new();
        for id in &ids {
            out.extend_from_slice(&self.store[id].data);
        }
        self.release(&ids);
        Some(out)
    }

    fn intern(&mut self, chunk: &[u8]) -> u64 {
        let mut hasher = DefaultHasher::new();
        hasher.write(chunk);
        let mut id = hasher.finish();
        loop {
            match self.store.entry(id) {
                Entry::Occupied(mut e) => {
                    if *e.get().data == *chunk {
                        e.get_mut().refs += 1;
                        return id;
                    }
                    // 64-bit hash collision: probe the next id.
                    id = id.wrapping_add(1);
                }
                Entry::Vacant(e) => {
                    self.stored_bytes += chunk.len();
                    e.insert(Chunk {
                        data: chunk.into(),
                        refs: 1,
                    });
                    return id;
                }
            }
        }
    }

    fn release(&mut self, ids: &[u64]) {
        for id in ids {
            if let Entry::Occupied(mut e) = self.store.entry(*id) {
                e.get_mut().refs -= 1;
                if e.get().refs == 0 {
                    self.stored_bytes -= e.get().data.len();
                    e.remove();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pseudo_json(seed: u32, len: usize) -> Vec<u8> {
        let mut s = seed;
        let mut out = b"[".to_vec();
        while out.len() < len {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
            out.extend_from_slice(format!("{},", (s >> 16) & 0xFF).as_bytes());
        }
        out
    }

    #[test]
    fn chunks_reassemble_and_respect_bounds() {
        let data = pseudo_json(1, 100_000);
        let parts: Vec<&[u8]> = chunks(&data).collect();
        assert_eq!(parts.concat(), data);
        for p in &parts[..parts.len() - 1] {
            assert!((MIN_CHUNK..=MAX_CHUNK).contains(&p.len()));
        }
    }

    #[test]
    fn push_pop_is_lifo_and_exact() {
        let mut r = Rewind::new(10);
        let a = pseudo_json(1, 50_000);
        let b = pseudo_json(2, 50_000);
        r.push(&a);
        r.push(&b);
        assert_eq!(r.pop().unwrap(), b);
        assert_eq!(r.pop().unwrap(), a);
        assert!(r.pop().is_none());
        assert_eq!(r.stored_bytes(), 0);
    }

    #[test]
    fn small_edit_shares_most_chunks() {
        let base = pseudo_json(7, 200_000);
        let mut edited = base.clone();
        // A digit-count change mid-blob shifts everything after it.
        edited.splice(100_000..100_001, b"123".iter().copied());
        let mut r = Rewind::new(10);
        r.push(&base);
        let one = r.stored_bytes();
        r.push(&edited);
        let extra = r.stored_bytes() - one;
        assert!(extra < 20_000, "edit cost {extra} bytes");
        assert_eq!(r.pop().unwrap(), edited);
        assert_eq!(r.pop().unwrap(), base);
    }

    #[test]
    fn capacity_evicts_oldest() {
        let mut r = Rewind::new(3);
        for i in 0..5 {
            r.push(&pseudo_json(i, 5_000));
        }
        assert_eq!(r.len(), 3);
        assert_eq!(r.pop().unwrap(), pseudo_json(4, 5_000));
        assert_eq!(r.pop().unwrap(), pseudo_json(3, 5_000));
        assert_eq!(r.pop().unwrap(), pseudo_json(2, 5_000));
        assert!(r.is_empty());
        assert_eq!(r.stored_bytes(), 0);
    }
}
