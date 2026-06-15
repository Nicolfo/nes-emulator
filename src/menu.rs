//! Home menu + settings screens, rendered NES-style into the 256x240 framebuffer.

use crate::config::{BUTTON_LABELS, Config};
use crate::font::{clear, draw_icon, draw_text, draw_text_centered, fill_rect, text_width};
use nes_emulator::ppu::WIDTH;

/// Vertical positions for the footer (divider + two hint lines) shared by every
/// menu. Derived from the visible window height `h` so the footer scales with
/// the running game instead of crowding a fixed 240-line layout.
fn footer_layout(h: i32) -> (i32, i32, i32, i32) {
    // (divider y, line 1 y, line 2 y, top of reserved band)
    (h - 28, h - 22, h - 12, h - 32)
}

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

pub fn render_home(frame: &mut [u8], h: i32, sel: usize, game_loaded: bool, error: Option<&str>) {
    clear(frame, BG);
    let cx = WIDTH as i32 / 2;

    // Header
    draw_text_centered(frame, 18, "RUST NES", TITLE_RED, 3);
    draw_text_centered(frame, 46, "EMULATOR", FG, 2);
    fill_rect(frame, 48, 66, 160, 1, DIM);
    let header_bottom = 70i32;

    let (footer_div, footer_l1, footer_l2, footer_top) = footer_layout(h);

    // Item list: centered as a block both horizontally (icon + gap + label) and
    // vertically in the gap between header and footer, so it fills whatever
    // window height the running game uses.
    let items = home_items(game_loaded);
    let spacing = 26i32;
    let block_h = items.len() as i32 * spacing;
    let items_top = header_bottom + ((footer_top - header_bottom) - block_h) / 2;
    let max_label = items
        .iter()
        .map(|it| text_width(it.label, 1))
        .max()
        .unwrap_or(0);
    let icon_x = cx - (24 + max_label) / 2;
    for (i, item) in items.iter().enumerate() {
        let y = items_top + i as i32 * spacing;
        let color = if i == sel { ACCENT } else { FG };
        if i == sel {
            draw_text(frame, icon_x - 16, y + 4, ">", ACCENT, 1);
        }
        draw_icon(frame, icon_x, y, item.icon, color);
        draw_text(frame, icon_x + 24, y + 4, item.label, color, 1);
    }

    if let Some(msg) = error {
        draw_text_centered(frame, footer_top - 8, msg, RED, 1);
    }

    fill_rect(frame, 48, footer_div, 160, 1, DIM);
    draw_text_centered(frame, footer_l1, "(C) 2026 NICOLFO", DIM, 1);
    draw_text_centered(frame, footer_l2, "BUILT IN RUST", DIM, 1);
}

/// Number of savestate slots offered in the F5/F7 picker.
pub const NUM_SLOTS: usize = 4;

/// Savestate slot picker. `saving` switches the title/hint between SAVE and
/// LOAD; `filled[i]` marks slots that already hold a snapshot.
pub fn render_slots(
    frame: &mut [u8],
    h: i32,
    saving: bool,
    sel: usize,
    filled: &[bool; NUM_SLOTS],
) {
    clear(frame, BG);

    let title = if saving { "SAVE STATE" } else { "LOAD STATE" };
    draw_text_centered(frame, 18, title, FG, 2);
    fill_rect(frame, 40, 40, 176, 1, DIM);
    let header_bottom = 46i32;

    let (footer_div, footer_l1, footer_l2, footer_top) = footer_layout(h);

    let spacing = 28i32;
    let block_h = NUM_SLOTS as i32 * spacing;
    let start_y = header_bottom + ((footer_top - header_bottom) - block_h) / 2;
    for (i, &is_filled) in filled.iter().enumerate() {
        let y = start_y + i as i32 * spacing;
        let selected = i == sel;
        let color = if selected { ACCENT } else { FG };
        if selected {
            draw_text(frame, 44, y + 4, ">", ACCENT, 1);
        }
        draw_text(frame, 64, y + 4, &format!("SLOT {}", i + 1), color, 1);
        let (status, scol) = if is_filled {
            ("SAVED", if selected { ACCENT } else { FG })
        } else {
            ("EMPTY", DIM)
        };
        draw_text(frame, 150, y + 4, status, scol, 1);
    }

    fill_rect(frame, 40, footer_div, 176, 1, DIM);
    let hint = if saving {
        "ENTER: SAVE   ESC: CANCEL"
    } else {
        "ENTER: LOAD   ESC: CANCEL"
    };
    draw_text_centered(frame, footer_l1, hint, DIM, 1);
    draw_text_centered(frame, footer_l2, "ARROWS: NAVIGATE", DIM, 1);
}

/// Settings rows: 0-7 buttons, 8 = player select, 9 = scale, 10 = overscan,
/// 11 = reset defaults, 12 = back.
pub const SETTINGS_ROWS: usize = 13;
pub const ROW_PLAYER: usize = 8;
pub const ROW_SCALE: usize = 9;
pub const ROW_OVERSCAN: usize = 10;
pub const ROW_RESET: usize = 11;
pub const ROW_BACK: usize = 12;

#[allow(clippy::needless_range_loop)]
pub fn render_settings(
    frame: &mut [u8],
    h: i32,
    cfg: &Config,
    sel: usize,
    waiting: bool,
    player: usize,
) {
    clear(frame, BG);

    draw_text_centered(frame, 8, "SETTINGS", FG, 2);
    fill_rect(frame, 40, 26, 176, 1, DIM);
    let header_bottom = 30i32;

    let (footer_div, footer_l1, footer_l2, footer_top) = footer_layout(h);

    // Button rows show the bindings for the currently selected player.
    let keys = if player == 0 { &cfg.keys } else { &cfg.keys_p2 };

    let spacing = 12i32;
    let block_h = SETTINGS_ROWS as i32 * spacing;
    let start_y = header_bottom + ((footer_top - header_bottom) - block_h) / 2;
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
                    Config::key_name(keys[i])
                };
                let value_color = if selected && waiting { RED } else { color };
                draw_text(frame, 120, y, &value, value_color, 1);
            }
            ROW_PLAYER => {
                draw_text(frame, 44, y, "EDIT PLAYER", color, 1);
                let v = format!("< {} >", player + 1);
                draw_text(frame, 160, y, &v, color, 1);
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

    fill_rect(frame, 40, footer_div, 176, 1, DIM);
    draw_text_centered(frame, footer_l1, "ENTER: CHANGE   ESC: BACK", DIM, 1);
    draw_text_centered(frame, footer_l2, "ARROWS: NAVIGATE", DIM, 1);
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
        render_home(
            &mut frame,
            HEIGHT as i32,
            0,
            true,
            Some("UNSUPPORTED MAPPER 4"),
        );
        write_bmp("menu_home.bmp", &frame, WIDTH, HEIGHT);
        let cfg = crate::config::Config::default();
        render_settings(&mut frame, HEIGHT as i32, &cfg, 2, false, 0);
        write_bmp("menu_settings.bmp", &frame, WIDTH, HEIGHT);
        let mut frame2 = vec![0u8; WIDTH * HEIGHT * 4];
        render_settings(&mut frame2, HEIGHT as i32, &cfg, 0, true, 0);
        write_bmp("menu_rebind.bmp", &frame2, WIDTH, HEIGHT);
    }

    /// Dump each menu as the player actually sees it with NTSC overscan crop:
    /// rendered at 240 then center-cropped to the 216-line window (the same
    /// slice `blit_menu` blits). Lets us eyeball clipping.
    #[test]
    #[ignore]
    fn dump_cropped_menus() {
        const CROP_H: usize = 216; // HEIGHT - OVERSCAN_TOP(16) - OVERSCAN_BOTTOM(8)
        // Menus now lay themselves out for the visible height, and blit_menu
        // copies the top CROP_H lines, so dump that same top slice.
        let top = |full: &[u8]| -> Vec<u8> { full[..CROP_H * WIDTH * 4].to_vec() };
        let cfg = crate::config::Config::default();
        let mut f = vec![0u8; WIDTH * HEIGHT * 4];

        render_home(&mut f, CROP_H as i32, 0, true, None);
        write_bmp("crop_home.bmp", &top(&f), WIDTH, CROP_H);
        render_settings(&mut f, CROP_H as i32, &cfg, 2, false, 0);
        write_bmp("crop_settings.bmp", &top(&f), WIDTH, CROP_H);
        render_slots(&mut f, CROP_H as i32, true, 1, &[true, false, true, false]);
        write_bmp("crop_slots.bmp", &top(&f), WIDTH, CROP_H);
    }
}
