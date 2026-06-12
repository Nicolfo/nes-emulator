//! Boot smoke tests for commercial ROMs in `testroms/`. Each test skips
//! itself when its ROM is absent, so CI (which has no ROMs) stays green.
//! A game "boots" if, after a few seconds of emulated time, the
//! framebuffer shows a real picture (more than a handful of distinct
//! pixel values) — a crashed or black-screen boot fails this.

use nes_emulator::nes::Nes;

fn boot_and_check(rom_name: &str, frames: u32, min_colors: usize) {
    let path = format!("testroms/{rom_name}");
    let Ok(rom) = std::fs::read(&path) else {
        eprintln!("{path} not found - skipping");
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
        distinct.len() > min_colors,
        "{rom_name}: framebuffer nearly uniform after {frames} frames ({} distinct colors)",
        distinct.len()
    );
}

#[test]
fn battletoads_boots() {
    // Mapper 7 (AxROM), CHR RAM, single-screen mirroring.
    boot_and_check("Battletoads (Europe).nes", 600, 8);
}

#[test]
fn smb3_boots() {
    // Mapper 4 (MMC3) regression check. The intro renders few colors until
    // the title screen comes up around frame 400.
    boot_and_check("Super Mario Bros. 3 (USA) (Rev 1).nes", 600, 8);
}

#[test]
fn castlevania3_boots() {
    // Mapper 5 (MMC5), scanline IRQ + ExRAM.
    boot_and_check("Castlevania III - Dracula's Curse (USA).nes", 600, 8);
}

#[test]
fn uncharted_waters_boots() {
    // Mapper 5 (MMC5) with PRG RAM banking. Black until ~frame 900, then a
    // story intro of white text on black (only ~8 distinct colors), so the
    // color bar is lower than for the other games.
    boot_and_check("Uncharted Waters (USA).nes", 1200, 4);
}
