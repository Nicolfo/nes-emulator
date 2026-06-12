//! Boot smoke tests for commercial ROMs sitting in the project root. Each
//! test skips itself when its ROM is absent, so CI (which has no ROMs)
//! stays green. A game "boots" if, after a few seconds of emulated time,
//! the framebuffer shows a real picture (more than a handful of distinct
//! pixel values) — a crashed or black-screen boot fails this.

use nes_emulator::nes::Nes;

fn boot_and_check(rom_name: &str, frames: u32) {
    let Ok(rom) = std::fs::read(rom_name) else {
        eprintln!("{rom_name} not found in project root - skipping");
        return;
    };
    let mut nes = Nes::new(&rom).unwrap_or_else(|e| panic!("{rom_name}: {e}"));
    for _ in 0..frames {
        nes.run_frame();
    }
    let mut distinct = std::collections::HashSet::new();
    for px in nes.framebuffer().chunks(4) {
        distinct.insert(px.to_vec());
    }
    assert!(
        distinct.len() > 8,
        "{rom_name}: framebuffer nearly uniform after {frames} frames ({} distinct colors)",
        distinct.len()
    );
}

#[test]
fn battletoads_boots() {
    // Mapper 7 (AxROM), CHR RAM, single-screen mirroring.
    boot_and_check("Battletoads (Europe).nes", 600);
}

#[test]
fn smb3_boots() {
    // Mapper 4 (MMC3) regression check. The intro renders few colors until
    // the title screen comes up around frame 400.
    boot_and_check("Super Mario Bros. 3 (USA) (Rev 1).nes", 600);
}
