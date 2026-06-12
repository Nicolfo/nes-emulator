//! Headless smoke test: load a ROM, run frames, report framebuffer activity.
//! Optionally taps buttons on given frames to get past menus.
//!
//! Usage: cargo run --release --example boot_check -- <rom> [frames] [frame:BTN,...]
//! BTN is one of A B S(elect) T(start) U D L R; each tap is held 5 frames.

use nes_emulator::controller::*;
use nes_emulator::nes::Nes;
use std::collections::HashSet;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: boot_check <rom> [frames] [frame:BTN,...]");
    let frames: u32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(300);
    let taps: Vec<(u32, u8)> = args
        .next()
        .map(|s| {
            s.split(',')
                .map(|v| {
                    let (frame, btn) = v.split_once(':').expect("expected frame:BTN");
                    let mask = match btn {
                        "A" => BTN_A,
                        "B" => BTN_B,
                        "S" => BTN_SELECT,
                        "T" => BTN_START,
                        "U" => BTN_UP,
                        "D" => BTN_DOWN,
                        "L" => BTN_LEFT,
                        "R" => BTN_RIGHT,
                        _ => panic!("unknown button {btn}"),
                    };
                    (frame.parse().unwrap(), mask)
                })
                .collect()
        })
        .unwrap_or_default();

    let data = std::fs::read(&path).expect("read ROM");
    let mut nes = Nes::new(&data).expect("load ROM");

    for f in 0..frames {
        // Hold each tapped button for 5 frames from its tap point.
        let mut state = 0u8;
        for &(s, mask) in &taps {
            if f >= s && f < s + 5 {
                state |= mask;
            }
        }
        nes.cpu.bus.controller1.state = state;
        nes.run_frame();
        if (f + 1) % 60 == 0 {
            let fb = nes.framebuffer();
            let colors: HashSet<&[u8]> = fb.chunks(4).collect();
            println!("frame {:4}: {} distinct colors", f + 1, colors.len());
        }
    }

    // Dump the final frame as a PPM for visual inspection.
    let fb = nes.framebuffer();
    let mut ppm = format!("P6\n256 240\n255\n").into_bytes();
    for px in fb.chunks(4) {
        ppm.extend_from_slice(&px[0..3]);
    }
    std::fs::write("boot_check.ppm", ppm).expect("write ppm");
    println!("wrote boot_check.ppm");
}
