//! Build script: on Windows, generates an .ico from the shared pixel art in
//! src/icon.rs and embeds it as the executable's icon resource, so the binary
//! shows the emulator icon in Explorer and the taskbar instead of the default.

#[cfg(windows)]
#[path = "src/icon.rs"]
mod icon;

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    #[cfg(windows)]
    embed_windows_icon();
}

#[cfg(windows)]
fn embed_windows_icon() {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let ico_path = std::path::Path::new(&out_dir).join("icon.ico");
    // 16, 32, 48, 64 and 256 px - the sizes Windows actually picks from.
    std::fs::write(&ico_path, build_ico(&[1, 2, 3, 4, 16])).expect("write icon.ico");
    winresource::WindowsResource::new()
        .set_icon(ico_path.to_str().expect("OUT_DIR not valid UTF-8"))
        .compile()
        .expect("embed Windows icon resource");
}

/// Builds an ICO file containing one 32-bit BMP image per scale factor.
#[cfg(windows)]
fn build_ico(scales: &[usize]) -> Vec<u8> {
    let images: Vec<(usize, Vec<u8>)> = scales
        .iter()
        .map(|&s| (icon::size(s), bmp_entry(s)))
        .collect();

    let mut out = Vec::new();
    // ICONDIR: reserved, type 1 (icon), image count.
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());

    // ICONDIRENTRY per image; width/height bytes use 0 to mean 256.
    let mut offset = 6 + 16 * images.len() as u32;
    for (side, data) in &images {
        let dim = if *side >= 256 { 0u8 } else { *side as u8 };
        out.push(dim);
        out.push(dim);
        out.push(0); // color count (not paletted)
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // color planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    out
}

/// One ICO image entry: BITMAPINFOHEADER + bottom-up BGRA pixels + AND mask.
#[cfg(windows)]
fn bmp_entry(scale: usize) -> Vec<u8> {
    let side = icon::size(scale);
    let rgba = icon::rgba(scale);

    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes()); // header size
    out.extend_from_slice(&(side as i32).to_le_bytes()); // width
    out.extend_from_slice(&(2 * side as i32).to_le_bytes()); // height (XOR + AND)
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&[0u8; 24]); // compression, sizes, palette: all zero

    // XOR data: BGRA rows, bottom-up.
    for y in (0..side).rev() {
        for x in 0..side {
            let i = (y * side + x) * 4;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    // AND mask: 1 bit per pixel, rows padded to 32 bits, all transparent
    // (alpha channel drives transparency for 32-bit icons).
    let mask_row_bytes = side.div_ceil(8).div_ceil(4) * 4;
    out.extend_from_slice(&vec![0u8; mask_row_bytes * side]);
    out
}
