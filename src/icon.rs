//! Pixel-art application icon (an NES controller), defined once here and used
//! both for the runtime window icon (main.rs) and for the Windows executable
//! icon resource generated at build time (build.rs).

/// 16x16 icon art. `B` = border, `G` = body, `D` = d-pad / select / start,
/// `R` = A/B buttons, anything else = transparent.
const ART: [&str; 16] = [
    "................",
    "................",
    "................",
    "................",
    ".BBBBBBBBBBBBBB.",
    ".BGGGGGGGGGGGGB.",
    ".BGGDGGGGGGGGGB.",
    ".BGDDDGGGGRGRGB.",
    ".BGGDGGDDGRGRGB.",
    ".BGGGGGGGGGGGGB.",
    ".BBBBBBBBBBBBBB.",
    "................",
    "................",
    "................",
    "................",
    "................",
];

/// Side length of the square icon at integer scale factor `scale`.
pub fn size(scale: usize) -> usize {
    ART.len() * scale
}

/// RGBA8 pixels of the icon scaled up by the integer factor `scale`
/// (nearest-neighbour, keeping the pixel-art look crisp).
pub fn rgba(scale: usize) -> Vec<u8> {
    let side = size(scale);
    let mut out = vec![0u8; side * side * 4];
    for (row, line) in ART.iter().enumerate() {
        for (col, ch) in line.bytes().enumerate() {
            let px: [u8; 4] = match ch {
                b'B' => [0x1a, 0x1a, 0x1a, 0xff],
                b'G' => [0xc9, 0xc9, 0xce, 0xff],
                b'D' => [0x3a, 0x3a, 0x3e, 0xff],
                b'R' => [0xe0, 0x3c, 0x3c, 0xff],
                _ => [0, 0, 0, 0],
            };
            for dy in 0..scale {
                for dx in 0..scale {
                    let i = ((row * scale + dy) * side + col * scale + dx) * 4;
                    out[i..i + 4].copy_from_slice(&px);
                }
            }
        }
    }
    out
}
