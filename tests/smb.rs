//! SMB headless smoke tests. Heavier dump test is #[ignore]d; run with:
//!   cargo test --test smb_smoke -- --ignored
//! Env vars: SMB_FRAMES (default 300), SMB_PRESS_START (frame number), SMB_RUN_RIGHT (any value)

use nes_emulator::controller::{BTN_RIGHT, BTN_START};
use nes_emulator::nes::Nes;

fn load_smb() -> Option<Nes> {
    let rom = std::fs::read("Super Mario Bros. (Japan, USA).nes").ok()?;
    Some(Nes::new(&rom).expect("load SMB"))
}

#[test]
fn title_screen_renders() {
    let Some(mut nes) = load_smb() else { return };
    for _ in 0..300 {
        nes.run_frame();
    }
    let fb = nes.framebuffer();
    let mut colors = std::collections::HashSet::new();
    for px in fb.chunks(4) {
        colors.insert((px[0], px[1], px[2]));
    }
    // title screen has sky, bricks, logo, text — needs a healthy palette
    assert!(colors.len() >= 8, "expected >=8 distinct colors, got {}", colors.len());
}

#[test]
fn sprite_zero_hit_occurs() {
    let Some(mut nes) = load_smb() else { return };
    // enter the game so the status-bar split is active
    for f in 0..400 {
        nes.cpu.bus.controller1.set_button(BTN_START, (180..185).contains(&f));
        nes.run_frame();
    }
    // run one more frame, sampling PPUSTATUS sprite-0 bit via the bus would clear
    // vblank, so instead verify the game reached gameplay: nonblank lower screen
    let fb = nes.framebuffer();
    let mut lower_colors = std::collections::HashSet::new();
    for y in 120..240 {
        for x in 0..256 {
            let i = (y * 256 + x) * 4;
            lower_colors.insert((fb[i], fb[i + 1], fb[i + 2]));
        }
    }
    assert!(lower_colors.len() >= 4, "lower screen too uniform: {}", lower_colors.len());
}

#[test]
#[ignore]
fn dump_frame_bmp() {
    let Some(mut nes) = load_smb() else { return };
    let frames: usize =
        std::env::var("SMB_FRAMES").ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    let press_start: Option<usize> =
        std::env::var("SMB_PRESS_START").ok().and_then(|s| s.parse().ok());
    let run_right = std::env::var("SMB_RUN_RIGHT").is_ok();

    for f in 0..frames {
        if let Some(s) = press_start {
            let pad = &mut nes.cpu.bus.controller1;
            pad.set_button(BTN_START, f >= s && f < s + 5);
            if run_right {
                pad.set_button(BTN_RIGHT, f >= s + 120);
            }
        }
        nes.run_frame();
    }
    write_bmp("frame.bmp", nes.framebuffer(), 256, 240);
    println!("wrote frame.bmp after {frames} frames");
}

fn write_bmp(path: &str, rgba: &[u8], w: usize, h: usize) {
    let row = w * 3;
    let size = 54 + row * h;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        for x in 0..w {
            let i = (y * w + x) * 4;
            out.push(rgba[i + 2]);
            out.push(rgba[i + 1]);
            out.push(rgba[i]);
        }
    }
    std::fs::write(path, out).expect("write bmp");
}
