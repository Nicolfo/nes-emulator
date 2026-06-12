//! Holy Mapperel (https://github.com/pinobatch/holy-mapperel) mapper test
//! ROMs from `testroms/`. Each ROM probes PRG/CHR banking, PRG RAM, and
//! mirroring for one board configuration and prints
//! "DETAILED TEST RESULT: NNNN" — 0000 means every check passed.
//!
//! The letters on screen use non-ASCII tiles but the digits and ':' are
//! ASCII, so instead of OCR we scan the PPU nametable RAM for the
//! `": NNNN"` byte pattern and assert the code is zero.
//!
//! Tests skip themselves when their ROM is absent, so CI (which has no
//! ROMs) stays green. Run with `--release`.

use nes_emulator::nes::Nes;

/// Runs the ROM, then scans both nametables for `':' ' ' d d d d` and
/// returns the four-digit result code.
fn run_and_read_result(rom_name: &str, frames: u32) -> Option<String> {
    let path = format!("testroms/{rom_name}");
    let Ok(rom) = std::fs::read(&path) else {
        eprintln!("{path} not found - skipping");
        return None;
    };
    let mut nes = Nes::new(&rom).unwrap_or_else(|e| panic!("{rom_name}: {e}"));
    for _ in 0..frames {
        nes.run_frame();
    }
    let vram = &nes.cpu.bus.ppu.vram;
    let code = vram.windows(6).find_map(|w| {
        let digits = &w[2..6];
        (w[0] == b':' && w[1] == b' ' && digits.iter().all(u8::is_ascii_digit))
            .then(|| String::from_utf8(digits.to_vec()).unwrap())
    });
    Some(code.unwrap_or_else(|| {
        panic!("{rom_name}: no 'RESULT: NNNN' line found on screen after {frames} frames")
    }))
}

fn check(rom_name: &str) {
    if let Some(code) = run_and_read_result(rom_name, 600) {
        assert_eq!(code, "0000", "{rom_name}: detailed test result {code}");
    }
}

macro_rules! hm_test {
    ($($name:ident: $rom:literal,)*) => {
        $(
            #[test]
            fn $name() {
                check($rom);
            }
        )*
    };
}

hm_test! {
    m0_p32k_c8k_v: "M0_P32K_C8K_V.nes",
    m0_p32k_cr32k_v: "M0_P32K_CR32K_V.nes",
    m0_p32k_cr8k_v: "M0_P32K_CR8K_V.nes",
    m1_p128k: "M1_P128K.nes",
    m1_p128k_c128k: "M1_P128K_C128K.nes",
    m1_p128k_c128k_s8k: "M1_P128K_C128K_S8K.nes",
    m1_p128k_c128k_w8k: "M1_P128K_C128K_W8K.nes",
    m1_p128k_c32k: "M1_P128K_C32K.nes",
    m1_p128k_c32k_s8k: "M1_P128K_C32K_S8K.nes",
    m1_p128k_c32k_w8k: "M1_P128K_C32K_W8K.nes",
    m1_p128k_cr8k: "M1_P128K_CR8K.nes",
    m1_p512k_cr8k_s32k: "M1_P512K_CR8K_S32K.nes",
    m1_p512k_cr8k_s8k: "M1_P512K_CR8K_S8K.nes",
    m1_p512k_s32k: "M1_P512K_S32K.nes",
    m1_p512k_s8k: "M1_P512K_S8K.nes",
    m2_p128k_cr8k_v: "M2_P128K_CR8K_V.nes",
    m2_p128k_v: "M2_P128K_V.nes",
    m3_p32k_c32k_h: "M3_P32K_C32K_H.nes",
    m4_p128k: "M4_P128K.nes",
    m4_p128k_cr32k: "M4_P128K_CR32K.nes",
    m4_p128k_cr8k: "M4_P128K_CR8K.nes",
    m4_p256k_c256k: "M4_P256K_C256K.nes",
    m7_p128k: "M7_P128K.nes",
    m7_p128k_cr8k: "M7_P128K_CR8K.nes",
    m9_p128k_c64k: "M9_P128K_C64K.nes",
    m11_p64k_c64k_v: "M11_P64K_C64K_V.nes",
    m11_p64k_cr32k_v: "M11_P64K_CR32K_V.nes",
    m66_p64k_c16k_v: "M66_P64K_C16K_V.nes",
    m69_p128k_c64k_s8k: "M69_P128K_C64K_S8K.nes",
    m69_p128k_c64k_w8k: "M69_P128K_C64K_W8K.nes",
}

// Unsupported mappers in testroms/, kept for when support lands:
// M10 (MMC4), M28 (Action 53), M34 (BNROM/NINA-001), M78.3 (Holy Diver),
// M118 (TxSROM), M180 (Crazy Climber).
