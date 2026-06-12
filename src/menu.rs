//! Home menu + settings screens, rendered NES-style into the 256x240 framebuffer.

use crate::config::{BUTTON_LABELS, Config};
use crate::font::{clear, draw_icon, draw_text, draw_text_centered, fill_rect};

pub const BG: [u8; 3] = [0x10, 0x12, 0x2E];
pub const FG: [u8; 3] = [0xC8, 0xC8, 0xD0];
pub const ACCENT: [u8; 3] = [0xF8, 0xC8, 0x30];
pub const DIM: [u8; 3] = [0x6A, 0x6E, 0x8A];
pub const RED: [u8; 3] = [0xE8, 0x50, 0x50];
pub const TITLE_RED: [u8; 3] = [0xE0, 0x4A, 0x3C];

#[rustfmt::skip]
pub const ICON_PLAY: [u16; 16] = [
    0x0000, 0x0000, 0x0C00, 0x0F00, 0x0FC0, 0x0FF0, 0x0FFC, 0x0FFC,
    0x0FF0, 0x0FC0, 0x0F00, 0x0C00, 0x0000, 0x0000, 0x0000, 0x0000,
];

#[rustfmt::skip]
pub const ICON_CART: [u16; 16] = [
    0x0000, 0x7FFE, 0x4002, 0x4FF2, 0x4FF2, 0x4FF2, 0x4002, 0x4002,
    0x4002, 0x4002, 0x47E2, 0x7FFE, 0x0000, 0x0000, 0x0000, 0x0000,
];

#[rustfmt::skip]
pub const ICON_GEAR: [u16; 16] = [
    0x0000, 0x03C0, 0x03C0, 0x0FF0, 0x1FF8, 0x3C3C, 0xF00F, 0xF00F,
    0xF00F, 0xF00F, 0x3C3C, 0x1FF8, 0x0FF0, 0x03C0, 0x03C0, 0x0000,
];

#[rustfmt::skip]
pub const ICON_POWER: [u16; 16] = [
    0x0000, 0x0180, 0x0180, 0x0DB0, 0x1998, 0x3186, 0x318C, 0x300C,
    0x300C, 0x300C, 0x1818, 0x0FF0, 0x03C0, 0x0000, 0x0000, 0x0000,
];

pub struct HomeItem {
    pub label: &'static str,
    pub icon: &'static [u16; 16],
    pub action: HomeAction,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HomeAction {
    Resume,
    LoadRom,
    Settings,
    Quit,
}

pub fn home_items(game_loaded: bool) -> Vec<HomeItem> {
    let mut items = Vec::new();
    if game_loaded {
        items.push(HomeItem {
            label: "RESUME",
            icon: &ICON_PLAY,
            action: HomeAction::Resume,
        });
    }
    items.push(HomeItem {
        label: "LOAD ROM",
        icon: &ICON_CART,
        action: HomeAction::LoadRom,
    });
    items.push(HomeItem {
        label: "SETTINGS",
        icon: &ICON_GEAR,
        action: HomeAction::Settings,
    });
    items.push(HomeItem {
        label: "QUIT",
        icon: &ICON_POWER,
        action: HomeAction::Quit,
    });
    items
}

pub fn render_home(frame: &mut [u8], sel: usize, game_loaded: bool, error: Option<&str>) {
    clear(frame, BG);

    draw_text_centered(frame, 22, "RUST NES", TITLE_RED, 3);
    draw_text_centered(frame, 48, "EMULATOR", FG, 2);
    fill_rect(frame, 48, 68, 160, 1, DIM);

    let items = home_items(game_loaded);
    let start_y = 88i32;
    for (i, item) in items.iter().enumerate() {
        let y = start_y + i as i32 * 26;
        let color = if i == sel { ACCENT } else { FG };
        if i == sel {
            draw_text(frame, 52, y + 4, ">", ACCENT, 1);
        }
        draw_icon(frame, 68, y, item.icon, color);
        draw_text(frame, 92, y + 4, item.label, color, 1);
    }

    if let Some(msg) = error {
        draw_text_centered(frame, 198, msg, RED, 1);
    }

    fill_rect(frame, 48, 209, 160, 1, DIM);
    draw_text_centered(frame, 214, "(C) 2026 NICOLFO", DIM, 1);
    draw_text_centered(frame, 226, "BUILT IN RUST", DIM, 1);
}

/// Settings rows: 0-7 buttons, 8 = scale, 9 = overscan, 10 = reset defaults,
/// 11 = back.
pub const SETTINGS_ROWS: usize = 12;
pub const ROW_SCALE: usize = 8;
pub const ROW_OVERSCAN: usize = 9;
pub const ROW_RESET: usize = 10;
pub const ROW_BACK: usize = 11;

#[allow(clippy::needless_range_loop)]
pub fn render_settings(frame: &mut [u8], cfg: &Config, sel: usize, waiting: bool) {
    clear(frame, BG);

    draw_text_centered(frame, 12, "SETTINGS", FG, 2);
    fill_rect(frame, 40, 32, 176, 1, DIM);

    let start_y = 40i32;
    let spacing = 13i32;
    for i in 0..SETTINGS_ROWS {
        let y = start_y + i as i32 * spacing;
        let selected = i == sel;
        let color = if selected { ACCENT } else { FG };
        if selected {
            draw_text(frame, 30, y, ">", ACCENT, 1);
        }
        match i {
            0..=7 => {
                draw_text(frame, 44, y, BUTTON_LABELS[i], color, 1);
                let value = if selected && waiting {
                    "PRESS A KEY...".to_string()
                } else {
                    Config::key_name(cfg.keys[i])
                };
                let value_color = if selected && waiting { RED } else { color };
                draw_text(frame, 120, y, &value, value_color, 1);
            }
            ROW_SCALE => {
                draw_text(frame, 44, y, "WINDOW SCALE", color, 1);
                draw_text(frame, 160, y, &format!("< {}X >", cfg.scale), color, 1);
            }
            ROW_OVERSCAN => {
                draw_text(frame, 44, y, "CROP OVERSCAN", color, 1);
                let v = if cfg.crop_overscan {
                    "< ON >"
                } else {
                    "< OFF >"
                };
                draw_text(frame, 160, y, v, color, 1);
            }
            ROW_RESET => draw_text(frame, 44, y, "RESET DEFAULTS", color, 1),
            ROW_BACK => draw_text(frame, 44, y, "BACK", color, 1),
            _ => {}
        }
    }

    fill_rect(frame, 40, 202, 176, 1, DIM);
    draw_text_centered(frame, 208, "ENTER: CHANGE   ESC: BACK", DIM, 1);
    draw_text_centered(frame, 220, "ARROWS: NAVIGATE", DIM, 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use nes_emulator::ppu::{HEIGHT, WIDTH};

    fn write_bmp(path: &str, rgba: &[u8], w: usize, h: usize) {
        let size = 54 + w * 3 * h;
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

    #[test]
    #[ignore]
    fn dump_menu_screens() {
        let mut frame = vec![0u8; WIDTH * HEIGHT * 4];
        render_home(&mut frame, 0, true, Some("UNSUPPORTED MAPPER 4"));
        write_bmp("menu_home.bmp", &frame, WIDTH, HEIGHT);
        let cfg = crate::config::Config::default();
        render_settings(&mut frame, &cfg, 2, false);
        write_bmp("menu_settings.bmp", &frame, WIDTH, HEIGHT);
        let mut frame2 = vec![0u8; WIDTH * HEIGHT * 4];
        render_settings(&mut frame2, &cfg, 0, true);
        write_bmp("menu_rebind.bmp", &frame2, WIDTH, HEIGHT);
    }
}
