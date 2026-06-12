//! Headless debugging tool: run a ROM for N frames, pressing Start at the
//! given frame numbers (held 12 frames), dumping a BMP screenshot every 60
//! frames.
//!
//!     cargo run --example framedump -- <rom> <frames> <out_prefix> [start_frame...]

use nes_emulator::nes::Nes;

const START: u8 = 0x08;

fn write_bmp(path: &str, fb: &[u8]) {
    let (w, h) = (256u32, 240u32);
    let row = w * 3;
    let pixel_bytes = row * h;
    let mut out = Vec::with_capacity(54 + pixel_bytes as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + pixel_bytes).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            out.extend_from_slice(&[fb[i + 2], fb[i + 1], fb[i]]); // BGR
        }
    }
    std::fs::write(path, out).unwrap();
}

fn main() {
    let mut args = std::env::args().skip(1);
    let rom_path = args
        .next()
        .expect("usage: framedump <rom> <frames> <prefix> [start_frame...]");
    let frames: u32 = args.next().expect("frame count").parse().unwrap();
    let prefix = args.next().expect("output prefix");
    let starts: Vec<u32> = args.map(|a| a.parse().unwrap()).collect();

    let rom = std::fs::read(&rom_path).expect("read rom");
    let mut nes = Nes::new(&rom).expect("load rom");
    let log = std::env::var("NES_MMC5_LOG").is_ok();
    for f in 0..frames {
        if log {
            eprintln!("frame {f}");
        }
        if starts.contains(&f) {
            nes.cpu.bus.controller1.set_button(START, true);
        }
        if starts.iter().any(|&s| f == s + 12) {
            nes.cpu.bus.controller1.set_button(START, false);
        }
        nes.run_frame();
        let every: u32 = std::env::var("NES_DUMP_EVERY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        if (f + 1) % every == 0 || f + 1 == frames || (log && f + 1 >= frames.saturating_sub(60)) {
            write_bmp(&format!("{prefix}_{:04}.bmp", f + 1), nes.framebuffer());
        }
        if log && f + 1 == frames {
            std::fs::write(format!("{prefix}_vram.bin"), nes.cpu.bus.ppu.vram).unwrap();
            std::fs::write(format!("{prefix}_oam.bin"), nes.cpu.bus.ppu.oam).unwrap();
        }
    }
}
