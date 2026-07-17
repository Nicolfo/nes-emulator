//! Robustness fuzzing for the ROM loader. `load_rom` parses fully untrusted
//! bytes (any file a user opens), so it must never panic - only ever return
//! `Ok` or a clean `Err`. These tests hammer it with corrupt and adversarial
//! headers and fail loudly (with the offending input) if any call panics.

use std::panic::catch_unwind;

use nes_emulator::cartridge::load_rom;
use nes_emulator::mapper::NtTarget;

/// Run `load_rom` on `data`; turn a panic into a test failure that names the
/// header bytes, so a regression points straight at the input that broke.
/// A loadable image must also survive its first accesses - reset-vector and
/// PRG reads, RAM pokes, pattern fetches, cart-routed nametable fetches -
/// because that's where degenerate sizes trip bank math (`% 0` or
/// out-of-bounds indexing), not in the parse. Everything is poked twice:
/// once at power-on defaults, and again after scribbling pseudo-random
/// values over the whole register space, so bank arithmetic also holds up
/// under arbitrary register states.
fn must_not_panic(data: &[u8]) {
    let header: Vec<u8> = data.iter().take(16).copied().collect();
    let result = catch_unwind(|| {
        let Ok((mut mapper, _, _)) = load_rom(data) else {
            return;
        };
        for scribble in [false, true] {
            if scribble {
                let mut seed = 0x9E37_79B9u32;
                for a in (0x4020..=0xFFFFu32).step_by(0x81) {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    mapper.cpu_write(a as u16, (seed >> 24) as u8);
                }
            }
            let _ = mapper.cpu_read(0xFFFC);
            let _ = mapper.cpu_read(0x8000);
            mapper.cpu_write(0x6123, 0xAB);
            let _ = mapper.prg_ram_read(0x6123);
            let _ = mapper.ppu_read(0x0000);
            let _ = mapper.ppu_read(0x1FFF);
            mapper.ppu_write(0x0000, 0xCD);
            // Nametable fetches reach the cartridge only when nt_target says
            // so (N163/Namco 175/340 CHR-as-nametable, Sunsoft-4 CHR-ROM
            // nametables) - mirroring the bus exercises those paths without
            // violating the ppu_read contract for CIRAM-routed boards.
            for nt in [0x2000u16, 0x2400, 0x2800, 0x2C00] {
                if mapper.nt_target(nt) == NtTarget::Cart {
                    let _ = mapper.ppu_read(nt | 0x03FF);
                    mapper.ppu_write(nt | 0x03FF, 0xEE);
                }
            }
        }
    });
    assert!(
        result.is_ok(),
        "load_rom panicked on header {header:02X?} (len {})",
        data.len()
    );
}

#[test]
fn random_headers_never_panic() {
    // Deterministic LCG; every input carries a valid magic so parsing gets
    // past the first gate and exercises the size/mapper logic underneath.
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as u32
    };
    for _ in 0..20_000 {
        let len = 16 + (next() as usize % 4096);
        let mut data = vec![0u8; len];
        data[0..4].copy_from_slice(b"NES\x1A");
        for b in &mut data[4..16] {
            *b = next() as u8;
        }
        // Randomly scribble into the body too, so trainer/PRG/CHR slices vary.
        for b in data[16..].iter_mut() {
            if next() & 7 == 0 {
                *b = next() as u8;
            }
        }
        must_not_panic(&data);
    }
}

#[test]
fn every_mapper_id_with_degenerate_sizes_never_panics() {
    // For each byte value as the mapper id, try a header that claims tiny or
    // zero PRG/CHR - the malformed shapes that trip naive bank math.
    for id in 0u16..=255 {
        for &(prg_banks, chr_banks) in &[(0u8, 0u8), (0, 1), (1, 0), (1, 1)] {
            let mut data = vec![0u8; 16 + 64 * 1024];
            data[0..4].copy_from_slice(b"NES\x1A");
            data[4] = prg_banks;
            data[5] = chr_banks;
            data[6] = ((id as u8) << 4) & 0xF0;
            data[7] = (id as u8) & 0xF0;
            must_not_panic(&data);
        }
    }
}

#[test]
fn nes2_exponent_sizes_never_panic() {
    // NES 2.0 exponent-multiplier size form (size nibble 0xF) can encode
    // gigantic sizes; the loader must reject them as truncated, not overflow.
    for lsb in 0u8..=255 {
        let mut data = vec![0u8; 16 + 1024];
        data[0..4].copy_from_slice(b"NES\x1A");
        data[7] = 0x08; // NES 2.0
        data[4] = lsb; // PRG LSB
        data[5] = lsb; // CHR LSB
        data[9] = 0xFF; // both size MSB nibbles = 0xF -> exponent form
        must_not_panic(&data);
    }
}

#[test]
fn nes2_ram_size_fields_never_panic() {
    // Bytes 10/11 carry RAM-size shift counts. Every value must produce a
    // mapper whose RAM window and CHR still serve accesses cleanly - the
    // round-up to 8KB in the loader is what keeps degenerate sizes safe.
    for b in 0u16..=255 {
        let mut data = vec![0u8; 16 + 32 * 1024 + 8 * 1024];
        data[0..4].copy_from_slice(b"NES\x1A");
        data[4] = 2;
        data[5] = 1;
        data[6] = 0x10; // mapper 1
        data[7] = 0x08; // NES 2.0
        data[10] = b as u8;
        data[11] = b as u8;
        let result = catch_unwind(|| {
            if let Ok((mut mapper, _, _)) = load_rom(&data) {
                mapper.cpu_write(0x6123, 0xAB);
                let _ = mapper.prg_ram_read(0x6123);
                mapper.ppu_write(0x1FFF, 0xCD);
                let _ = mapper.ppu_read(0x1FFF);
            }
        });
        assert!(result.is_ok(), "RAM-size byte {b:02X} panicked");
    }
}

#[test]
fn minimal_valid_nrom_loads() {
    // Sanity floor: a well-formed 16KB NROM still parses cleanly.
    let mut data = vec![0u8; 16 + 16 * 1024];
    data[0..4].copy_from_slice(b"NES\x1A");
    data[4] = 1; // 16KB PRG
    assert!(load_rom(&data).is_ok());
}
