//! Home menu + settings screens, rendered NES-style into the 256x240 framebuffer.

use crate::config::{BUTTON_LABELS, Config, NUM_BINDINGS};
use crate::font::{clear, draw_icon, draw_text, draw_text_centered, fill_rect, text_width};
use nes_emulator::controller::InputDevice;
use nes_emulator::nsf::NsfInfo;
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

#[rustfmt::skip]
pub const ICON_KEY: [u16; 16] = [
    0x0000, 0x0000, 0x0000, 0x0000, 0x3C00, 0x7E00, 0xE7FF, 0xC3FF,
    0xE733, 0x7E33, 0x3C00, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000,
];

pub struct HomeItem {
    pub label: &'static str,
    pub icon: &'static [u16; 16],
    pub action: HomeAction,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HomeAction {
    Resume,
    LoadRom,
    Cheats,
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
    if game_loaded {
        items.push(HomeItem {
            label: "CHEATS",
            icon: &ICON_KEY,
            action: HomeAction::Cheats,
        });
    }
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
    // Shrink the row pitch when the list would not fit the window.
    let spacing = ((footer_top - header_bottom) / items.len() as i32).clamp(18, 26);
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

/// Settings pages.
pub const PAGE_CONTROLS: usize = 0;
pub const PAGE_OPTIONS: usize = 1;
pub const NUM_PAGES: usize = 2;

/// One selectable settings row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SetRow {
    /// Page switcher (CONTROLS / OPTIONS).
    Page,
    /// Which player's bindings the page shows.
    Player,
    /// Key binding, index into [`BUTTON_LABELS`].
    Bind(usize),
    Scale,
    Overscan,
    Volume,
    FastForward,
    Input,
    Region,
    Rewind,
    Reset,
    Back,
}

/// Rows shown on settings `page`, top to bottom.
pub fn settings_rows(page: usize) -> Vec<SetRow> {
    let mut rows = vec![SetRow::Page];
    if page == PAGE_CONTROLS {
        rows.push(SetRow::Player);
        rows.extend((0..NUM_BINDINGS).map(SetRow::Bind));
    } else {
        rows.extend([
            SetRow::Scale,
            SetRow::Overscan,
            SetRow::Volume,
            SetRow::FastForward,
            SetRow::Input,
            SetRow::Region,
            SetRow::Rewind,
            SetRow::Reset,
        ]);
    }
    rows.push(SetRow::Back);
    rows
}

pub fn input_device_name(d: InputDevice) -> &'static str {
    match d {
        InputDevice::Standard => "2 PADS",
        InputDevice::FourScore => "FOUR SCORE",
        InputDevice::Zapper => "ZAPPER",
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "< ON >" } else { "< OFF >" }
}

pub fn render_settings(
    frame: &mut [u8],
    h: i32,
    cfg: &Config,
    page: usize,
    sel: usize,
    waiting: bool,
    player: usize,
) {
    clear(frame, BG);

    draw_text_centered(frame, 8, "SETTINGS", FG, 2);
    fill_rect(frame, 40, 26, 176, 1, DIM);
    let header_bottom = 30i32;

    let (footer_div, footer_l1, footer_l2, footer_top) = footer_layout(h);

    let rows = settings_rows(page);
    let spacing = ((footer_top - header_bottom) / rows.len() as i32).clamp(10, 12);
    let block_h = rows.len() as i32 * spacing;
    let start_y = header_bottom + ((footer_top - header_bottom) - block_h) / 2;
    for (i, row) in rows.iter().enumerate() {
        let y = start_y + i as i32 * spacing;
        let selected = i == sel;
        let color = if selected { ACCENT } else { FG };
        if selected {
            draw_text(frame, 30, y, ">", ACCENT, 1);
        }
        let (label, value) = match *row {
            SetRow::Page => (
                "PAGE",
                Some(
                    if page == PAGE_OPTIONS {
                        "< OPTIONS >"
                    } else {
                        "< CONTROLS >"
                    }
                    .to_string(),
                ),
            ),
            SetRow::Player => ("EDIT PLAYER", Some(format!("< {} >", player + 1))),
            SetRow::Bind(b) => {
                if selected && waiting {
                    draw_text(frame, 120, y, "PRESS A KEY...", RED, 1);
                } else {
                    let key = Config::key_name(cfg.binding(player, b));
                    draw_text(frame, 120, y, &key, color, 1);
                }
                (BUTTON_LABELS[b], None)
            }
            SetRow::Scale => ("WINDOW SCALE", Some(format!("< {}X >", cfg.scale))),
            SetRow::Overscan => ("CROP OVERSCAN", Some(on_off(cfg.crop_overscan).to_string())),
            SetRow::Volume => ("VOLUME", Some(format!("< {}% >", cfg.volume))),
            SetRow::FastForward => ("FAST FORWARD", Some(format!("< {}X >", cfg.ff_speed))),
            SetRow::Input => (
                "INPUT",
                Some(format!("< {} >", input_device_name(cfg.input_device))),
            ),
            SetRow::Region => ("REGION", Some(format!("< {} >", cfg.region.label()))),
            SetRow::Rewind => ("REWIND", Some(on_off(cfg.rewind).to_string())),
            SetRow::Reset => ("RESET DEFAULTS", None),
            SetRow::Back => ("BACK", None),
        };
        draw_text(frame, 44, y, label, color, 1);
        if let Some(v) = value {
            // Values start at a common column; wide ones (FOUR SCORE) shift
            // left so they never run off the right edge.
            let x = (248 - text_width(&v, 1)).min(160);
            draw_text(frame, x, y, &v, color, 1);
        }
    }

    fill_rect(frame, 40, footer_div, 176, 1, DIM);
    draw_text_centered(frame, footer_l1, "ENTER: CHANGE   ESC: BACK", DIM, 1);
    draw_text_centered(frame, footer_l2, "ARROWS: NAVIGATE / ADJUST", DIM, 1);
}

/// Maximum characters in a typed cheat code (`AAAA?CC:VV`).
pub const CHEAT_MAX_LEN: usize = 10;

/// Cheat list screen. Rows are the codes, then ADD CODE, then BACK.
/// `entry` is the code being typed (Some while in entry mode); `msg` shows
/// a parse error or hint above the footer.
pub fn render_cheats(
    frame: &mut [u8],
    h: i32,
    cheats: &[(String, bool)],
    sel: usize,
    entry: Option<&str>,
    msg: Option<&str>,
) {
    clear(frame, BG);
    draw_text_centered(frame, 8, "CHEATS", FG, 2);
    fill_rect(frame, 40, 26, 176, 1, DIM);
    let header_bottom = 32i32;
    let (footer_div, footer_l1, footer_l2, footer_top) = footer_layout(h);

    let total = cheats.len() + 2;
    let spacing = 12i32;
    let list_bottom = footer_top - 12; // room for the message line
    let visible = (((list_bottom - header_bottom) / spacing).max(1) as usize).min(total);
    // Scroll so the selection stays in view.
    let first = sel.saturating_sub(visible - 1).min(total - visible);
    for (slot, i) in (first..first + visible).enumerate() {
        let y = header_bottom + slot as i32 * spacing;
        let selected = i == sel;
        let color = if selected { ACCENT } else { FG };
        if selected {
            draw_text(frame, 30, y, ">", ACCENT, 1);
        }
        if let Some((code, enabled)) = cheats.get(i) {
            draw_text(frame, 44, y, code, color, 1);
            let (v, c) = if *enabled {
                ("ON", color)
            } else {
                ("OFF", DIM)
            };
            draw_text(frame, 196, y, v, c, 1);
        } else if i == cheats.len() {
            match entry {
                Some(text) => draw_text(frame, 44, y, &format!("{text}_"), RED, 1),
                None => draw_text(frame, 44, y, "ADD CODE", color, 1),
            }
        } else {
            draw_text(frame, 44, y, "BACK", color, 1);
        }
    }
    if first > 0 {
        draw_text(frame, 222, header_bottom, "^", DIM, 1);
    }
    if first + visible < total {
        draw_text(frame, 222, list_bottom - spacing, "V", DIM, 1);
    }
    if let Some(m) = msg {
        draw_text_centered(frame, list_bottom, m, RED, 1);
    }

    fill_rect(frame, 40, footer_div, 176, 1, DIM);
    let (l1, l2) = if entry.is_some() {
        ("TYPE CODE   ENTER: ADD", "BACKSPACE: ERASE  ESC: CANCEL")
    } else {
        ("ENTER: TOGGLE/ADD  DEL: REMOVE", "ESC: BACK")
    };
    draw_text_centered(frame, footer_l1, l1, DIM, 1);
    draw_text_centered(frame, footer_l2, l2, DIM, 1);
}

/// Truncate `s` to what fits in `max_chars` 8px glyphs.
fn fit(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max_chars.saturating_sub(2)).collect();
        t.push_str("..");
        t
    }
}

/// NSF music player screen: tune metadata, track number and play time.
pub fn render_nsf(frame: &mut [u8], h: i32, info: &NsfInfo, seconds: u64) {
    clear(frame, BG);
    draw_text_centered(frame, 14, "NSF PLAYER", TITLE_RED, 2);
    fill_rect(frame, 40, 34, 176, 1, DIM);
    let (footer_div, footer_l1, footer_l2, _) = footer_layout(h);

    let unknown = |s: &str| {
        if s.trim().is_empty() || s == "<?>" {
            "-".to_string()
        } else {
            fit(s, 28)
        }
    };
    draw_text_centered(frame, 50, &unknown(&info.title), ACCENT, 1);
    draw_text_centered(frame, 64, &unknown(&info.artist), FG, 1);
    draw_text_centered(frame, 76, &unknown(&info.copyright), DIM, 1);

    let track = format!("< TRACK {} / {} >", info.current_song + 1, info.total_songs);
    let mid = (78 + footer_div) / 2;
    draw_text_centered(frame, mid - 12, &track, FG, 2);
    let time = format!("{}:{:02}", seconds / 60, seconds % 60);
    draw_text_centered(frame, mid + 12, &time, DIM, 1);

    let chips: Vec<&str> = [
        (nes_emulator::nsf::EXP_VRC6, "VRC6"),
        (nes_emulator::nsf::EXP_VRC7, "VRC7"),
        (nes_emulator::nsf::EXP_FDS, "FDS"),
        (nes_emulator::nsf::EXP_MMC5, "MMC5"),
        (nes_emulator::nsf::EXP_N163, "N163"),
        (nes_emulator::nsf::EXP_5B, "5B"),
    ]
    .iter()
    .filter(|(bit, _)| info.expansion & bit != 0)
    .map(|&(_, name)| name)
    .collect();
    if !chips.is_empty() {
        draw_text_centered(frame, mid + 26, &format!("+ {}", chips.join(" ")), DIM, 1);
    }

    fill_rect(frame, 40, footer_div, 176, 1, DIM);
    draw_text_centered(frame, footer_l1, "LEFT/RIGHT: TRACK  F3: RESTART", DIM, 1);
    draw_text_centered(frame, footer_l2, "ESC: MENU", DIM, 1);
}

/// Draw a short status line (e.g. "FAST FORWARD 4X") over the top-left of
/// the running game, on a dark box so it reads on any background.
pub fn draw_osd(frame: &mut [u8], text: &str) {
    let w = text_width(text, 1);
    fill_rect(frame, 2, 2, w + 6, 12, BG);
    draw_text(frame, 5, 4, text, ACCENT, 1);
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
        render_settings(&mut frame, HEIGHT as i32, &cfg, PAGE_CONTROLS, 2, false, 0);
        write_bmp("menu_settings.bmp", &frame, WIDTH, HEIGHT);
        render_settings(&mut frame, HEIGHT as i32, &cfg, PAGE_OPTIONS, 2, false, 0);
        write_bmp("menu_options.bmp", &frame, WIDTH, HEIGHT);
        let mut frame2 = vec![0u8; WIDTH * HEIGHT * 4];
        render_settings(&mut frame2, HEIGHT as i32, &cfg, PAGE_CONTROLS, 3, true, 0);
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
        render_settings(&mut f, CROP_H as i32, &cfg, PAGE_CONTROLS, 2, false, 0);
        write_bmp("crop_settings.bmp", &top(&f), WIDTH, CROP_H);
        render_settings(&mut f, CROP_H as i32, &cfg, PAGE_OPTIONS, 4, false, 0);
        write_bmp("crop_options.bmp", &top(&f), WIDTH, CROP_H);
        let cheats = vec![("SXIOPO".to_string(), true), ("075A:09".to_string(), false)];
        render_cheats(
            &mut f,
            CROP_H as i32,
            &cheats,
            2,
            Some("YEUZ"),
            Some("BAD LETTER Q"),
        );
        write_bmp("crop_cheats.bmp", &top(&f), WIDTH, CROP_H);
        render_slots(&mut f, CROP_H as i32, true, 1, &[true, false, true, false]);
        write_bmp("crop_slots.bmp", &top(&f), WIDTH, CROP_H);
    }

    // --- Logic ---------------------------------------------------------------

    #[test]
    fn resume_and_cheats_only_when_game_loaded() {
        let loaded = home_items(true);
        assert_eq!(loaded[0].action, HomeAction::Resume);
        assert!(loaded.iter().any(|i| i.action == HomeAction::Cheats));
        let empty = home_items(false);
        assert!(
            empty
                .iter()
                .all(|i| i.action != HomeAction::Resume && i.action != HomeAction::Cheats)
        );
        // Everything else is identical regardless of game state.
        assert_eq!(loaded.len(), empty.len() + 2);
    }

    #[test]
    fn footer_scales_with_height() {
        let (div, l1, l2, top) = footer_layout(240);
        assert!(top < div && div < l1 && l1 < l2 && l2 < 240);
        // Taller window pushes the whole footer band down by the same delta.
        let (div2, ..) = footer_layout(260);
        assert_eq!(div2 - div, 20);
    }

    // --- Render smoke (pixels, no file I/O) -----------------------------------

    fn blank() -> Vec<u8> {
        vec![0u8; WIDTH * HEIGHT * 4]
    }

    /// Every render must paint the background, draw *something* on top, and
    /// never touch a byte outside the framebuffer.
    fn assert_painted(f: &[u8]) {
        assert_eq!(&f[0..3], &BG, "background not cleared");
        assert!(
            f.chunks(4).any(|p| p[..3] != BG),
            "nothing drawn over background"
        );
    }

    #[test]
    fn render_home_every_selection() {
        for game_loaded in [true, false] {
            let n = home_items(game_loaded).len();
            for sel in 0..n {
                let mut f = blank();
                let err = if sel == 0 {
                    Some("UNSUPPORTED MAPPER 99")
                } else {
                    None
                };
                render_home(&mut f, HEIGHT as i32, sel, game_loaded, err);
                assert_painted(&f);
            }
        }
    }

    #[test]
    fn render_slots_save_and_load() {
        for saving in [true, false] {
            for sel in 0..NUM_SLOTS {
                let mut f = blank();
                render_slots(
                    &mut f,
                    HEIGHT as i32,
                    saving,
                    sel,
                    &[true, false, false, true],
                );
                assert_painted(&f);
            }
        }
    }

    #[test]
    fn render_settings_all_rows_both_players() {
        let mut cfg = crate::config::Config::default();
        for device in [
            InputDevice::Standard,
            InputDevice::FourScore,
            InputDevice::Zapper,
        ] {
            cfg.input_device = device;
            for page in 0..NUM_PAGES {
                for player in [0, 1] {
                    for sel in 0..settings_rows(page).len() {
                        for waiting in [true, false] {
                            let mut f = blank();
                            render_settings(
                                &mut f,
                                HEIGHT as i32,
                                &cfg,
                                page,
                                sel,
                                waiting,
                                player,
                            );
                            assert_painted(&f);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn settings_pages_cover_every_binding_and_option() {
        let controls = settings_rows(PAGE_CONTROLS);
        for b in 0..NUM_BINDINGS {
            assert!(controls.contains(&SetRow::Bind(b)));
        }
        let options = settings_rows(PAGE_OPTIONS);
        for r in [
            SetRow::Volume,
            SetRow::FastForward,
            SetRow::Input,
            SetRow::Rewind,
        ] {
            assert!(options.contains(&r));
        }
        for page in 0..NUM_PAGES {
            let rows = settings_rows(page);
            assert_eq!(rows[0], SetRow::Page);
            assert_eq!(*rows.last().unwrap(), SetRow::Back);
        }
    }

    #[test]
    fn render_cheats_scrolls_long_lists() {
        let many: Vec<(String, bool)> = (0..40)
            .map(|i| (format!("{:04X}:00", i), i % 2 == 0))
            .collect();
        for h in [HEIGHT as i32, 216] {
            for sel in [0, 20, 40, 41] {
                let mut f = blank();
                render_cheats(&mut f, h, &many, sel, None, None);
                assert_painted(&f);
            }
            let mut f = blank();
            render_cheats(&mut f, h, &[], 0, Some("SXIO"), Some("BAD"));
            assert_painted(&f);
        }
    }

    #[test]
    fn render_nsf_screen() {
        let info = NsfInfo {
            title: "A VERY LONG TITLE THAT DOES NOT FIT ON ONE LINE".into(),
            artist: String::new(),
            copyright: "<?>".into(),
            total_songs: 12,
            current_song: 2,
            expansion: nes_emulator::nsf::EXP_VRC6 | nes_emulator::nsf::EXP_FDS,
        };
        for h in [HEIGHT as i32, 216] {
            let mut f = blank();
            render_nsf(&mut f, h, &info, 125);
            assert_painted(&f);
        }
        assert_eq!(fit("ABCDEFGHIJ", 6), "ABCD..");
    }

    #[test]
    fn osd_draws_on_cropped_frame() {
        let mut f = vec![0u8; WIDTH * 216 * 4];
        draw_osd(&mut f, "FAST FORWARD 4X");
        assert!(f.iter().any(|&b| b != 0));
    }
}
