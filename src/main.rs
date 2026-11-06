// Release builds use the Windows GUI subsystem so launching the .exe (from
// Explorer or a shortcut) doesn't pop up a console window behind the emulator.
// Debug builds keep the console so NES_*_LOG / trace output stays visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod font;
mod gamepad;
mod icon;
mod menu;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pixels::{Pixels, SurfaceTexture};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorIcon, Fullscreen, Window, WindowId};

use nes_emulator::cartridge::Region;
use nes_emulator::cheats;
use nes_emulator::controller::InputDevice;
use nes_emulator::nes::Nes;
use nes_emulator::ppu::{HEIGHT, WIDTH};
use nes_emulator::rewind::Rewind;

use config::{BUTTON_MASKS, Config, FIRST_TURBO, MAX_FF_SPEED, MIN_FF_SPEED, PLAYERS};
use gamepad::{MAX_PADS, PadState};
use menu::{HomeAction, NUM_PAGES, PAGE_CONTROLS, SetRow, home_items, settings_rows};

/// Scanlines hidden by NTSC overscan. Top crop is deeper: raster-split
/// games (e.g. Castlevania III) finish their scanline-IRQ bank switch a
/// line or two into the frame, so garbage can extend past the classic 8.
const OVERSCAN_TOP: usize = 16;
const OVERSCAN_BOTTOM: usize = 8;

// NTSC: 89342 PPU dots per frame at 5,369,318 dots/sec = 60.0988 FPS
const FRAME_PERIOD: Duration = Duration::from_nanos(16_639_267);
// PAL: 106392 PPU dots per frame at 5,320,342.5 dots/sec = 50.0070 FPS
const PAL_FRAME_PERIOD: Duration = Duration::from_nanos(19_997_200);

/// Turbo buttons alternate pressed/released every `TURBO_PERIOD / 2`
/// frames: 15 presses per second at 60 FPS, slow enough that games which
/// poll input every other frame still see every release.
const TURBO_PERIOD: u64 = 4;
/// Take a rewind snapshot every this many emulated frames...
const REWIND_INTERVAL: u64 = 2;
/// ...keeping this many (20 s of history at 60 FPS). Rewinding plays one
/// snapshot per displayed frame, i.e. backwards at 2x speed.
const REWIND_CAPACITY: usize = 600;
/// How long a status message stays on screen.
const OSD_DURATION: Duration = Duration::from_secs(2);

fn frame_period(nes: &Nes) -> Duration {
    match nes.region() {
        Region::Ntsc => FRAME_PERIOD,
        // Dendy: same 312x341-dot frame on the same master clock as PAL.
        Region::Pal | Region::Dendy => PAL_FRAME_PERIOD,
    }
}

enum View {
    Home {
        sel: usize,
    },
    Settings {
        page: usize,
        sel: usize,
        waiting: bool,
        player: usize,
    },
    /// Savestate slot picker shown over the paused game. `saving` selects
    /// F5 (save) vs F7 (load) behaviour; `sel` is the highlighted slot.
    SlotPicker {
        saving: bool,
        sel: usize,
    },
    /// Cheat list. `entry` holds the code being typed; `msg` a parse error.
    Cheats {
        sel: usize,
        entry: Option<String>,
        msg: Option<String>,
    },
    Running,
}

/// One line of the cheat list, persisted to `<rom>.cht`.
struct CheatEntry {
    code: String,
    enabled: bool,
}

struct App {
    window: Option<Arc<Window>>,
    pixels: Option<Pixels<'static>>,
    nes: Option<Nes>,
    view: View,
    cfg: Config,
    error: Option<String>,
    next_frame: Instant,
    audio: Option<audio::Audio>,
    /// .sav file next to the loaded ROM; battery RAM is written back here.
    sav_path: Option<PathBuf>,
    /// Physical gamepads (None if gilrs couldn't start - keyboard only).
    gamepads: Option<gamepad::Gamepads>,
    /// ROM file of the running game (to rebuild it on a region change).
    rom_path: Option<PathBuf>,
    /// Per-player button masks from the keyboard and the gamepads (players
    /// 1-4), kept apart so one input source never clears buttons the other
    /// is holding. The controllers see their union.
    kb_mask: [u8; 4],
    /// Keyboard turbo buttons held per player (A/B bits, pulsed).
    kb_turbo: [u8; 4],
    pads: [PadState; MAX_PADS],
    /// Timestamp of the last left-button press, for double-click detection.
    last_click: Option<Instant>,
    /// Emulated frames since the game loaded; drives the turbo phase and
    /// the rewind snapshot cadence.
    frame_count: u64,
    /// Fast-forward key (Tab) held.
    fast_forward: bool,
    /// Rewind key (Backspace) held.
    rewinding: bool,
    rewind: Rewind,
    /// Sound muted with F6 (not persisted; the volume setting is).
    muted: bool,
    cheats: Vec<CheatEntry>,
    /// Last cursor position in physical window pixels (Zapper aim).
    cursor: Option<(f64, f64)>,
    /// Zapper trigger (left mouse) and "shoot off screen" (right mouse).
    zapper_trigger: bool,
    zapper_offscreen: bool,
    /// Transient status line drawn over the game.
    osd: Option<(String, Instant)>,
    /// FDS: the side most recently requested with F8 (the drive reports
    /// None while a swap is in progress).
    fds_side: usize,
    /// NSF: frame_count when the current track started (play-time display).
    track_start: u64,
}

/// Max gap between two left clicks to count as a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Restore <rom>.sav into battery RAM (no-op without a battery) and return
/// the path so it can be written back later.
fn restore_battery_ram(nes: &mut Nes, rom_path: &Path) -> PathBuf {
    let sav = rom_path.with_extension("sav");
    if let Ok(data) = std::fs::read(&sav) {
        nes.load_battery_ram(&data);
    }
    sav
}

/// Parse a `.cht` file: one code per line, `-CODE` = disabled, `#` comments.
fn parse_cheat_file(text: &str) -> Vec<CheatEntry> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.strip_prefix('-') {
            Some(code) => (code.trim(), false),
            None => (l, true),
        })
        .filter(|(code, _)| cheats::parse(code).is_ok())
        .map(|(code, enabled)| CheatEntry {
            code: code.to_ascii_uppercase(),
            enabled,
        })
        .collect()
}

fn format_cheat_file(list: &[CheatEntry]) -> String {
    let mut out = String::from("# nes-emulator cheats: one code per line, '-' prefix = disabled\n");
    for c in list {
        if !c.enabled {
            out.push('-');
        }
        out.push_str(&c.code);
        out.push('\n');
    }
    out
}

impl App {
    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Drop any queued audio so a pause/state-change doesn't replay stale samples.
    fn clear_audio(&self) {
        if let Some(a) = &self.audio {
            a.clear();
        }
    }

    /// Show `msg` over the game for a couple of seconds.
    fn flash(&mut self, msg: impl Into<String>) {
        self.osd = Some((msg.into(), Instant::now()));
    }

    /// Push the volume setting (or mute) to the audio device.
    fn apply_volume(&self) {
        if let Some(a) = &self.audio {
            let gain = if self.muted {
                0.0
            } else {
                self.cfg.volume as f32 / 100.0
            };
            a.set_volume(gain);
        }
    }

    /// Button masks for players 1-4 this frame: keyboard OR gamepad, plus
    /// turbo buttons during the "pressed" half of the turbo cycle.
    fn frame_inputs(&self) -> [u8; 4] {
        let turbo_on = (self.frame_count / (TURBO_PERIOD / 2)).is_multiple_of(2);
        let mut masks = [0u8; 4];
        for (p, m) in masks.iter_mut().enumerate() {
            let pad = self.pads[p];
            *m = self.kb_mask[p] | pad.buttons;
            if turbo_on {
                *m |= self.kb_turbo[p] | pad.turbo;
            }
        }
        masks
    }

    /// Zapper aim in NES pixels, from the cursor position; None = off screen.
    fn zapper_aim(&self) -> Option<(i32, i32)> {
        if self.zapper_offscreen {
            return None;
        }
        let (x, y) = self.cursor?;
        let pixels = self.pixels.as_ref()?;
        let (px, py) = pixels.window_pos_to_pixel((x as f32, y as f32)).ok()?;
        let (skip, _) = self.crop();
        Some((px as i32, (py + skip) as i32))
    }

    /// Forget keys/buttons held when the game stops receiving input (menu
    /// opened), since their release events will go to the menu instead.
    fn release_held_inputs(&mut self) {
        self.kb_mask = [0; 4];
        self.kb_turbo = [0; 4];
        self.fast_forward = false;
        self.rewinding = false;
        self.zapper_trigger = false;
        self.zapper_offscreen = false;
    }

    /// Crosshair over the game while the Zapper is plugged in.
    fn update_cursor(&self) {
        if let Some(w) = &self.window {
            let zapper = matches!(self.view, View::Running)
                && self.nes.is_some()
                && self.cfg.input_device == InputDevice::Zapper;
            w.set_cursor(if zapper {
                CursorIcon::Crosshair
            } else {
                CursorIcon::Default
            });
        }
    }

    /// Leave a menu or the slot picker and resume play. Resetting next_frame
    /// stops the catch-up loop from fast-forwarding over the paused time.
    fn resume_game(&mut self) {
        self.view = View::Running;
        self.next_frame = Instant::now();
        self.update_cursor();
        self.redraw();
    }

    /// Path of savestate `slot` (0-based): `<rom>.stateN`, next to the ROM.
    fn state_path(&self, slot: usize) -> Option<PathBuf> {
        self.sav_path
            .as_ref()
            .map(|p| p.with_extension(format!("state{}", slot + 1)))
    }

    /// Cheat list file next to the ROM.
    fn cheat_path(&self) -> Option<PathBuf> {
        self.sav_path.as_ref().map(|p| p.with_extension("cht"))
    }

    /// Which of the NUM_SLOTS savestate slots already hold a snapshot.
    fn slot_states(&self) -> [bool; menu::NUM_SLOTS] {
        let mut out = [false; menu::NUM_SLOTS];
        for (slot, exists) in out.iter_mut().enumerate() {
            *exists = self.state_path(slot).is_some_and(|p| p.exists());
        }
        out
    }

    /// Snapshot the running machine to savestate `slot` (F5).
    fn save_state(&mut self, slot: usize) {
        let (Some(nes), Some(path)) = (&self.nes, self.state_path(slot)) else {
            return;
        };
        match nes.save_state().and_then(|data| {
            atomic_write(&path, &data).map_err(|e| format!("write {}: {e}", path.display()))
        }) {
            Ok(()) => {
                eprintln!("saved state to {}", path.display());
                self.flash(format!("SAVED SLOT {}", slot + 1));
            }
            Err(e) => {
                eprintln!("save state failed: {e}");
                self.flash("SAVE FAILED");
            }
        }
    }

    /// Restore savestate `slot` into the running machine (F7).
    fn load_state(&mut self, slot: usize) {
        let Some(path) = self.state_path(slot) else {
            return;
        };
        let Some(nes) = &mut self.nes else { return };
        if !path.exists() {
            eprintln!("no savestate at {}", path.display());
            self.flash(format!("SLOT {} IS EMPTY", slot + 1));
            return;
        }
        // A savestate is a local file but still untrusted input: a hand-edited
        // blob can carry inconsistent state. load_state validates magic/region,
        // and the catch_unwind contains any panic from restoring bad mapper data.
        match std::fs::read(&path)
            .map_err(|e| format!("read {}: {e}", path.display()))
            .and_then(|data| {
                catch_unwind(AssertUnwindSafe(|| nes.load_state(&data)))
                    .unwrap_or_else(|p| Err(format!("savestate rejected: {}", panic_msg(p))))
            }) {
            Ok(()) => {
                eprintln!("loaded state from {}", path.display());
                self.flash(format!("LOADED SLOT {}", slot + 1));
                self.clear_audio();
                self.redraw();
            }
            Err(e) => {
                eprintln!("load state failed: {e}");
                self.flash("LOAD FAILED");
            }
        }
    }

    /// Write battery RAM to the .sav file; no-op without battery/ROM.
    fn save_battery_ram(&self) {
        let (Some(nes), Some(path)) = (&self.nes, &self.sav_path) else {
            return;
        };
        if let Some(ram) = nes.battery_ram()
            && let Err(e) = atomic_write(path, ram)
        {
            eprintln!("failed to write {}: {e}", path.display());
        }
    }

    /// Install the enabled cheats into the running machine.
    fn apply_cheats(&mut self) {
        let parsed: Vec<_> = self
            .cheats
            .iter()
            .filter(|c| c.enabled)
            .filter_map(|c| cheats::parse(&c.code).ok())
            .collect();
        if let Some(nes) = &mut self.nes {
            nes.set_cheats(&parsed);
        }
    }

    fn save_cheats(&self) {
        let Some(path) = self.cheat_path() else {
            return;
        };
        if self.cheats.is_empty() {
            let _ = std::fs::remove_file(&path);
        } else if let Err(e) = atomic_write(&path, format_cheat_file(&self.cheats).as_bytes()) {
            eprintln!("failed to write {}: {e}", path.display());
        }
    }

    fn load_rom_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("NES games and music", &["nes", "fds", "nsf"])
            .add_filter("NES ROM", &["nes"])
            .add_filter("Famicom Disk System", &["fds"])
            .add_filter("NSF music", &["nsf"])
            .set_title("Load NES ROM")
            .pick_file();
        if let Some(path) = picked {
            self.open_rom(&path);
        }
    }

    /// Load the ROM at `path` and start it; on failure stay in the menu with
    /// the error shown.
    fn open_rom(&mut self, path: &Path) {
        let rom = match std::fs::read(path) {
            Ok(rom) => rom,
            Err(e) => {
                self.error = Some(format!("READ FAILED: {e}"));
                return;
            }
        };
        let is_fds = Nes::is_fds_image(&rom)
            || path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("fds"));
        let built = if is_fds {
            // The FDS needs Nintendo's BIOS, which can't ship with the
            // emulator: the user supplies disksys.rom.
            match config::find_fds_bios(path) {
                Ok(bios) => safe_build(|| Nes::new_fds(&rom, &bios)),
                Err(tried) => {
                    eprintln!("FDS BIOS not found; looked for:");
                    for p in tried {
                        eprintln!("  {}", p.display());
                    }
                    Err("FDS BIOS DISKSYS.ROM NOT FOUND".into())
                }
            }
        } else {
            let region = self.cfg.region.forced();
            safe_build(|| Nes::new_with_region(&rom, region))
        };
        match built {
            Ok(nes) => self.install_game(nes, path),
            Err(e) => self.error = Some(e),
        }
    }

    /// Swap in a freshly built machine for the ROM at `path`: flush the
    /// outgoing game's save, restore this one's .sav and cheats, apply the
    /// host settings and start running.
    fn install_game(&mut self, mut nes: Nes, path: &Path) {
        self.save_battery_ram(); // flush the outgoing game first
        if let Some(a) = &self.audio {
            nes.set_sample_rate(a.sample_rate as f64);
        }
        nes.set_input_device(self.cfg.input_device);
        self.sav_path = Some(restore_battery_ram(&mut nes, path));
        self.rom_path = Some(path.to_path_buf());
        self.nes = Some(nes);
        self.error = None;
        self.frame_count = 0;
        self.track_start = 0;
        self.fds_side = 0;
        self.rewind.clear();
        self.cheats = self
            .cheat_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| parse_cheat_file(&t))
            .unwrap_or_default();
        self.apply_cheats();
        if let Some(w) = &self.window {
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("NES");
            w.set_title(&format!("NES Emulator - {name}"));
        }
        self.start_running();
    }

    /// Rebuild a running cartridge game so a REGION change takes effect now.
    /// The battery save is flushed first; the game restarts from power-on.
    /// FDS and NSF ignore the setting, so they keep running.
    fn restart_for_region(&mut self) {
        let Some(path) = self.rom_path.clone() else {
            return;
        };
        let is_cart = self
            .nes
            .as_ref()
            .is_some_and(|n| n.fds_side_count().is_none() && n.nsf_info().is_none());
        if !is_cart {
            return;
        }
        let view = std::mem::replace(&mut self.view, View::Running);
        self.open_rom(&path);
        // Stay on the settings page; the window keeps the new game's size.
        self.view = view;
        self.update_cursor();
        self.clear_audio();
        if let Some(nes) = &self.nes {
            let name = match nes.region() {
                Region::Ntsc => "NTSC",
                Region::Pal => "PAL",
                Region::Dendy => "DENDY",
            };
            self.flash(format!("RESTARTED AS {name}"));
        }
    }

    fn start_running(&mut self) {
        self.view = View::Running;
        self.next_frame = Instant::now();
        self.apply_view_size();
        self.update_cursor();
    }

    /// Open the home menu over the paused game.
    fn open_menu(&mut self) {
        self.save_battery_ram();
        self.release_held_inputs();
        self.view = View::Home { sel: 0 };
        self.apply_view_size();
        self.clear_audio();
        self.update_cursor();
        // Make sure the pause menu comes to the front and takes input.
        if let Some(w) = &self.window {
            w.focus_window();
        }
        self.redraw();
    }

    /// (rows cropped from the top, visible height) for the current view.
    fn crop(&self) -> (usize, usize) {
        if matches!(self.view, View::Running) && self.overscan_cropped() {
            (OVERSCAN_TOP, HEIGHT - OVERSCAN_TOP - OVERSCAN_BOTTOM)
        } else {
            (0, HEIGHT)
        }
    }

    /// Whether the loaded game is shown with NTSC overscan cropped.
    fn overscan_cropped(&self) -> bool {
        self.cfg.crop_overscan
            && self
                .nes
                .as_ref()
                .is_some_and(|n| n.region() == Region::Ntsc)
    }

    /// Visible height of the running game, independent of the current view.
    /// The window tracks this even in menus so opening the menu never resizes
    /// the OS window.
    fn running_height(&self) -> usize {
        if self.nes.is_some() && self.overscan_cropped() {
            HEIGHT - OVERSCAN_TOP - OVERSCAN_BOTTOM
        } else {
            HEIGHT
        }
    }

    /// Match pixel buffer and window size to the running view. Buffer and
    /// window keep the same height across all views, so the menu fills the
    /// window edge-to-edge (no letterbox bars) and opening it never resizes
    /// the OS window. Menus that assume the full 240 lines are fitted in
    /// `blit_menu`.
    fn apply_view_size(&mut self) {
        let h = self.running_height();
        if let Some(p) = &mut self.pixels {
            let _ = p.resize_buffer(WIDTH as u32, h as u32);
        }
        // A fullscreen window must not be snapped back to the scaled size, or
        // loading a ROM / changing scale would silently drop out of fullscreen.
        if let Some(w) = &self.window
            && w.fullscreen().is_none()
        {
            let _ = w.request_inner_size(LogicalSize::new(
                (WIDTH as u32 * self.cfg.scale) as f64,
                (h as u32 * self.cfg.scale) as f64,
            ));
        }
    }

    /// Flip OS borderless fullscreen on the active monitor (no title bar).
    /// Reading the live `fullscreen()` state keeps the toggle correct even if
    /// the OS dropped us out on its own. `apply_view_size` skips its resize
    /// while fullscreen so loading a ROM doesn't snap the window back.
    fn toggle_fullscreen(&mut self) {
        if let Some(w) = &self.window {
            let mode = w
                .fullscreen()
                .is_none()
                .then_some(Fullscreen::Borderless(None));
            w.set_fullscreen(mode);
        }
    }

    /// Treat the second of two quick left clicks as a double-click and toggle
    /// fullscreen. Resets the timer so a third click starts a fresh pair.
    fn handle_click(&mut self) {
        let now = Instant::now();
        let double = self
            .last_click
            .is_some_and(|t| now.duration_since(t) <= DOUBLE_CLICK);
        if double {
            self.last_click = None;
            self.toggle_fullscreen();
        } else {
            self.last_click = Some(now);
        }
    }

    /// Whether mouse buttons currently drive the Zapper (so clicks must not
    /// toggle fullscreen).
    fn zapper_active(&self) -> bool {
        matches!(self.view, View::Running)
            && self.nes.is_some()
            && self.cfg.input_device == InputDevice::Zapper
    }

    fn mouse_input(&mut self, button: MouseButton, state: ElementState) {
        let pressed = state.is_pressed();
        if self.zapper_active() {
            match button {
                MouseButton::Left => self.zapper_trigger = pressed,
                // Right button fires off screen (the "reload" gesture some
                // games use, e.g. pointing away and pulling the trigger).
                MouseButton::Right => {
                    self.zapper_offscreen = pressed;
                    self.zapper_trigger = pressed;
                }
                _ => {}
            }
        } else if button == MouseButton::Left && pressed {
            self.handle_click();
        }
    }

    fn home_key(&mut self, code: KeyCode, event_loop: &ActiveEventLoop) {
        let View::Home { sel } = &mut self.view else {
            return;
        };
        let items = home_items(self.nes.is_some());
        match code {
            KeyCode::ArrowUp => *sel = (*sel + items.len() - 1) % items.len(),
            KeyCode::ArrowDown => *sel = (*sel + 1) % items.len(),
            KeyCode::Enter | KeyCode::Space => {
                self.error = None;
                match items[*sel].action {
                    HomeAction::Resume => self.start_running(),
                    HomeAction::LoadRom => self.load_rom_dialog(),
                    HomeAction::Cheats => {
                        self.view = View::Cheats {
                            sel: 0,
                            entry: None,
                            msg: None,
                        }
                    }
                    HomeAction::Settings => {
                        self.view = View::Settings {
                            page: PAGE_CONTROLS,
                            sel: 0,
                            waiting: false,
                            player: 0,
                        }
                    }
                    HomeAction::Quit => event_loop.exit(),
                }
            }
            // ESC backs out of the menu: resume the game if one is loaded,
            // otherwise (nothing to resume) it quits.
            KeyCode::Escape => {
                if self.nes.is_some() {
                    self.start_running();
                } else {
                    event_loop.exit();
                }
            }
            _ => {}
        }
        self.redraw();
    }

    fn settings_key(&mut self, code: KeyCode) {
        let View::Settings {
            mut page,
            mut sel,
            mut waiting,
            mut player,
        } = self.view
        else {
            return;
        };
        let rows = settings_rows(page);
        sel = sel.min(rows.len() - 1);

        if waiting {
            // capture next key as the new binding (Escape cancels)
            if code != KeyCode::Escape
                && let SetRow::Bind(b) = rows[sel]
            {
                // Hotkeys (F-keys, Tab, Backspace...) can't be buttons.
                if !config::is_reserved(code) {
                    self.cfg.rebind(player, b, code);
                }
                self.cfg.save();
            }
            waiting = false;
        } else {
            let delta = match code {
                KeyCode::ArrowLeft => Some(-1),
                KeyCode::ArrowRight | KeyCode::Enter | KeyCode::Space => Some(1),
                _ => None,
            };
            let activate = matches!(code, KeyCode::Enter | KeyCode::Space);
            match code {
                KeyCode::ArrowUp => sel = (sel + rows.len() - 1) % rows.len(),
                KeyCode::ArrowDown => sel = (sel + 1) % rows.len(),
                KeyCode::Escape => {
                    self.view = View::Home { sel: 0 };
                    return self.redraw();
                }
                _ => {
                    let Some(delta) = delta else { return };
                    match rows[sel] {
                        SetRow::Page => {
                            page = (page as i32 + delta).rem_euclid(NUM_PAGES as i32) as usize
                        }
                        SetRow::Player => {
                            player = (player as i32 + delta).rem_euclid(PLAYERS as i32) as usize
                        }
                        SetRow::Bind(_) if activate => waiting = true,
                        SetRow::Bind(_) => {}
                        SetRow::Back if activate => {
                            self.view = View::Home { sel: 0 };
                            return self.redraw();
                        }
                        SetRow::Back => {}
                        row => self.adjust_option(row, delta, activate),
                    }
                }
            }
        }
        self.view = View::Settings {
            page,
            sel,
            waiting,
            player,
        };
        self.redraw();
    }

    /// Change an OPTIONS-page setting. Left/right step by `delta`; Enter
    /// (`activate`) steps forward and wraps around.
    fn adjust_option(&mut self, row: SetRow, delta: i32, activate: bool) {
        fn step(v: i32, delta: i32, lo: i32, hi: i32, wrap: bool) -> i32 {
            let n = v + delta;
            if wrap && n > hi { lo } else { n.clamp(lo, hi) }
        }
        match row {
            SetRow::Scale => {
                self.cfg.scale = step(self.cfg.scale as i32, delta, 1, 5, activate) as u32;
                self.apply_view_size();
            }
            SetRow::Overscan => self.cfg.crop_overscan = !self.cfg.crop_overscan,
            SetRow::Volume => {
                self.cfg.volume = step(self.cfg.volume as i32, delta * 10, 0, 100, activate) as u8;
                self.apply_volume();
            }
            SetRow::FastForward => {
                self.cfg.ff_speed = step(
                    self.cfg.ff_speed as i32,
                    delta,
                    MIN_FF_SPEED as i32,
                    MAX_FF_SPEED as i32,
                    activate,
                ) as u8;
            }
            SetRow::Input => {
                const DEVICES: [InputDevice; 3] = [
                    InputDevice::Standard,
                    InputDevice::FourScore,
                    InputDevice::Zapper,
                ];
                let i = DEVICES
                    .iter()
                    .position(|&d| d == self.cfg.input_device)
                    .unwrap_or(0) as i32;
                self.cfg.input_device = DEVICES[(i + delta).rem_euclid(3) as usize];
                if let Some(nes) = &mut self.nes {
                    nes.set_input_device(self.cfg.input_device);
                }
            }
            SetRow::Region => {
                self.cfg.region = self.cfg.region.step(delta);
                self.cfg.save();
                self.restart_for_region();
                return;
            }
            SetRow::Rewind => {
                self.cfg.rewind = !self.cfg.rewind;
                if !self.cfg.rewind {
                    self.rewind.clear();
                }
            }
            SetRow::Reset if activate => {
                self.cfg = Config {
                    scale: self.cfg.scale,
                    ..Config::default()
                };
                self.apply_volume();
                if let Some(nes) = &mut self.nes {
                    nes.set_input_device(self.cfg.input_device);
                }
            }
            _ => return,
        }
        self.cfg.save();
    }

    fn cheats_key(&mut self, code: KeyCode, text: Option<&str>) {
        let View::Cheats { sel, entry, msg } = &mut self.view else {
            return;
        };
        let total = self.cheats.len() + 2;
        if let Some(buf) = entry {
            match code {
                KeyCode::Escape => *entry = None,
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Enter | KeyCode::NumpadEnter => match cheats::parse(buf) {
                    Ok(_) => {
                        let code = buf.trim().to_ascii_uppercase();
                        if !self.cheats.iter().any(|c| c.code == code) {
                            self.cheats.push(CheatEntry {
                                code,
                                enabled: true,
                            });
                        }
                        *entry = None;
                        *msg = None;
                        *sel = self.cheats.len(); // keep the cursor on ADD CODE
                        self.save_cheats();
                        self.apply_cheats();
                    }
                    Err(e) => *msg = Some(e),
                },
                _ => {
                    for ch in text.unwrap_or("").chars() {
                        let ch = ch.to_ascii_uppercase();
                        if (ch.is_ascii_alphanumeric() || ch == ':' || ch == '?')
                            && buf.len() < menu::CHEAT_MAX_LEN
                        {
                            buf.push(ch);
                        }
                    }
                }
            }
            return self.redraw();
        }
        *msg = None;
        match code {
            KeyCode::ArrowUp => *sel = (*sel + total - 1) % total,
            KeyCode::ArrowDown => *sel = (*sel + 1) % total,
            KeyCode::Enter | KeyCode::Space => {
                if let Some(c) = self.cheats.get_mut(*sel) {
                    c.enabled = !c.enabled;
                    self.save_cheats();
                    self.apply_cheats();
                } else if *sel == self.cheats.len() {
                    *entry = Some(String::new());
                } else {
                    self.view = View::Home { sel: 0 };
                }
            }
            KeyCode::Delete | KeyCode::Backspace if *sel < self.cheats.len() => {
                self.cheats.remove(*sel);
                self.save_cheats();
                self.apply_cheats();
            }
            KeyCode::Escape => self.view = View::Home { sel: 0 },
            _ => return,
        }
        self.redraw();
    }

    fn running_key(&mut self, code: KeyCode, pressed: bool, repeat: bool) {
        if repeat {
            return;
        }
        match code {
            KeyCode::Escape if pressed => return self.open_menu(),
            // Savestate slots: F5 opens the save picker, F7 the load picker.
            // The picker pauses the game (about_to_wait only ticks in Running).
            KeyCode::F5 | KeyCode::F7 => {
                if pressed && self.nes.is_some() {
                    self.view = View::SlotPicker {
                        saving: code == KeyCode::F5,
                        sel: 0,
                    };
                    self.release_held_inputs();
                    self.clear_audio();
                    self.update_cursor();
                    self.redraw();
                }
                return;
            }
            // F3 is the console RESET button: re-enter the cartridge's reset
            // vector with RAM/VRAM intact. Not rebindable (like F5/F7).
            // For an NSF it restarts the current track.
            KeyCode::F3 => {
                if pressed && let Some(nes) = &mut self.nes {
                    if let Some(info) = nes.nsf_info() {
                        nes.nsf_play(info.current_song);
                        self.track_start = self.frame_count;
                    } else {
                        nes.reset();
                        self.flash("RESET");
                    }
                }
                return;
            }
            // F8 flips the FDS disk to the next side (eject, then insert).
            KeyCode::F8 => {
                if pressed && let Some(nes) = &mut self.nes {
                    if let Some(count) = nes.fds_side_count() {
                        // Step from the side actually in the drive (a
                        // savestate or rewind may have changed it); fall
                        // back to the last request while a swap is pending.
                        let cur = nes.fds_current_side().unwrap_or(self.fds_side);
                        self.fds_side = (cur + 1) % count;
                        nes.fds_insert(Some(self.fds_side));
                        let side = self.fds_side;
                        self.flash(format!(
                            "DISK {} SIDE {}",
                            side / 2 + 1,
                            if side.is_multiple_of(2) { 'A' } else { 'B' }
                        ));
                    } else {
                        self.flash("NOT A DISK SYSTEM GAME");
                    }
                }
                return;
            }
            // NSF player: left/right change track.
            KeyCode::ArrowLeft | KeyCode::ArrowRight
                if self.nes.as_ref().is_some_and(|n| n.nsf_info().is_some()) =>
            {
                if pressed && let Some(nes) = &mut self.nes {
                    let info = nes.nsf_info().expect("checked above");
                    let n = info.total_songs;
                    let song = if code == KeyCode::ArrowLeft {
                        (info.current_song + n - 1) % n
                    } else {
                        (info.current_song + 1) % n
                    };
                    nes.nsf_play(song);
                    self.track_start = self.frame_count;
                    self.rewind.clear();
                }
                return;
            }
            KeyCode::F6 => {
                if pressed {
                    self.muted = !self.muted;
                    self.apply_volume();
                    self.flash(if self.muted { "MUTED" } else { "SOUND ON" });
                }
                return;
            }
            // Hold Tab to fast-forward, Backspace to rewind.
            KeyCode::Tab => {
                self.fast_forward = pressed;
                return;
            }
            KeyCode::Backspace => {
                if pressed && !self.cfg.rewind {
                    self.flash("REWIND IS OFF (SETTINGS)");
                }
                self.rewinding = pressed && self.cfg.rewind;
                if !pressed {
                    // Resume from the rewound point without a timing burst.
                    self.next_frame = Instant::now();
                }
                return;
            }
            _ => {}
        }
        // The keyboard owns the keyboard half of each player's button mask;
        // the gamepad half is refreshed every frame in about_to_wait.
        for player in 0..PLAYERS {
            for (b, &bit) in BUTTON_MASKS.iter().enumerate() {
                if self.cfg.binding(player, b) == code {
                    let mask = if b < FIRST_TURBO {
                        &mut self.kb_mask[player]
                    } else {
                        &mut self.kb_turbo[player]
                    };
                    set_mask(mask, bit, pressed);
                }
            }
        }
    }

    fn slot_key(&mut self, code: KeyCode) {
        let View::SlotPicker { saving, sel } = &mut self.view else {
            return;
        };
        match code {
            // Arrows move the highlight; fall through to the redraw below.
            KeyCode::ArrowUp => *sel = (*sel + menu::NUM_SLOTS - 1) % menu::NUM_SLOTS,
            KeyCode::ArrowDown => *sel = (*sel + 1) % menu::NUM_SLOTS,
            KeyCode::Enter | KeyCode::Space => {
                let (saving, slot) = (*saving, *sel);
                if saving {
                    self.save_state(slot);
                } else {
                    self.load_state(slot);
                }
                return self.resume_game();
            }
            KeyCode::Escape => return self.resume_game(),
            _ => return,
        }
        self.redraw();
    }

    /// Emulate one frame with this frame's inputs, taking a rewind snapshot
    /// first when one is due. Err carries a panic message.
    fn emulate_frame(&mut self) -> Result<(), String> {
        let inputs = self.frame_inputs();
        let aim = self.zapper_aim();
        let trigger = self.zapper_trigger;
        // Fast-forward spaces snapshots out by its speed, so history still
        // covers the same wall-clock time and capturing doesn't eat the
        // frame budget the speed-up needs.
        let interval = if self.fast_forward {
            REWIND_INTERVAL * self.cfg.ff_speed as u64
        } else {
            REWIND_INTERVAL
        };
        let capture = self.cfg.rewind && self.frame_count.is_multiple_of(interval);
        let nes = self.nes.as_mut().expect("emulate_frame needs a game");
        nes.set_buttons(inputs);
        nes.set_zapper(aim, trigger);
        if capture {
            self.rewind.push(&nes.snapshot());
        }
        catch_unwind(AssertUnwindSafe(|| nes.run_frame())).map_err(panic_msg)?;
        self.frame_count += 1;
        Ok(())
    }

    /// Step back one rewind snapshot and render the frame that follows it.
    /// Holds on the oldest snapshot once history runs out.
    fn rewind_frame(&mut self) -> Result<(), String> {
        let Some(snap) = self.rewind.pop() else {
            return Ok(());
        };
        let nes = self.nes.as_mut().expect("rewind_frame needs a game");
        if let Err(e) = nes.restore_snapshot(&snap) {
            eprintln!("rewind failed: {e}");
            self.rewind.clear();
            return Ok(());
        }
        catch_unwind(AssertUnwindSafe(|| nes.run_frame())).map_err(panic_msg)?;
        // Keep the snapshot we just showed so history doesn't drain one
        // frame per press, and releasing resumes from exactly here.
        if self.rewind.is_empty() {
            self.rewind.push(&snap);
        }
        Ok(())
    }

    /// Status line for the running game, if any.
    fn osd_text(&self) -> Option<String> {
        if self.rewinding {
            return Some(if self.rewind.len() <= 1 {
                "<< REWIND (NO MORE HISTORY)".into()
            } else {
                "<< REWIND".into()
            });
        }
        if self.fast_forward {
            return Some(format!(">> {}X", self.cfg.ff_speed));
        }
        self.osd
            .as_ref()
            .filter(|(_, t)| t.elapsed() < OSD_DURATION)
            .map(|(m, _)| m.clone())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // 64x64: big enough for taskbar + title bar; ignored on macOS, where
        // window icons don't exist (the dock icon would need an .app bundle).
        let window_icon = winit::window::Icon::from_rgba(
            icon::rgba(4),
            icon::size(4) as u32,
            icon::size(4) as u32,
        )
        .ok();
        let attrs = Window::default_attributes()
            .with_title("NES Emulator")
            .with_window_icon(window_icon)
            .with_inner_size(LogicalSize::new(
                (WIDTH as u32 * self.cfg.scale) as f64,
                (HEIGHT as u32 * self.cfg.scale) as f64,
            ));
        // The desktop environment matches a window to its launcher entry (and
        // thus its taskbar icon) by app_id on Wayland / WM_CLASS on X11. Both
        // must equal the .desktop file's basename ("nes-emulator") for the
        // installed icon to render, since winit's RGBA window icon is X11-only.
        // Gated to the targets where winit exposes its Wayland/X11 extension
        // modules (Linux + BSDs); other Unixes lack them and wouldn't compile.
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
        ))]
        let attrs = {
            use winit::platform::wayland::WindowAttributesExtWayland;
            use winit::platform::x11::WindowAttributesExtX11;
            let attrs =
                WindowAttributesExtWayland::with_name(attrs, "nes-emulator", "nes-emulator");
            WindowAttributesExtX11::with_name(attrs, "nes-emulator", "nes-emulator")
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let size = window.inner_size();
        let surface = SurfaceTexture::new(size.width, size.height, window.clone());
        let pixels =
            Pixels::new(WIDTH as u32, HEIGHT as u32, surface).expect("create pixel buffer");
        self.window = Some(window);
        self.pixels = Some(pixels);
        // CLI boot starts directly in Running: sync buffer/window to the
        // overscan crop, which is otherwise only applied on view changes.
        self.apply_view_size();
        self.update_cursor();
        self.next_frame = Instant::now();
        self.redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(pixels) = &mut self.pixels {
                    let _ = pixels.resize_surface(size.width.max(1), size.height.max(1));
                }
                self.redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some((position.x, position.y));
            }
            WindowEvent::CursorLeft { .. } => self.cursor = None,
            WindowEvent::MouseInput { state, button, .. } => self.mouse_input(button, state),
            // Losing focus drops key-up events: release everything held.
            WindowEvent::Focused(false) => self.release_held_inputs(),
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state.is_pressed();
                // F11 toggles fullscreen in every view (not rebindable). Escape
                // is left to the per-view handlers (opens the menu / backs out)
                // and never exits fullscreen.
                if pressed && !event.repeat && code == KeyCode::F11 {
                    self.toggle_fullscreen();
                    return;
                }
                match self.view {
                    View::Running => self.running_key(code, pressed, event.repeat),
                    View::Home { .. } if pressed => self.home_key(code, event_loop),
                    View::Settings { .. } if pressed => self.settings_key(code),
                    View::SlotPicker { .. } if pressed => self.slot_key(code),
                    View::Cheats { .. } if pressed => self.cheats_key(code, event.text.as_deref()),
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                let (skip, h) = self.crop();
                // slot_states() borrows &self, which would clash with the
                // &mut self.pixels borrow below, so snapshot it up front.
                // Only the picker reads it.
                let picker_slots =
                    matches!(self.view, View::SlotPicker { .. }).then(|| self.slot_states());
                let osd = self.osd_text();
                let nsf = self.nes.as_ref().and_then(|n| n.nsf_info());
                let fps = match self.nes.as_ref().map(|n| n.region()) {
                    Some(Region::Pal | Region::Dendy) => 50,
                    _ => 60,
                };
                let nsf_secs = (self.frame_count - self.track_start) / fps;
                let cheat_rows: Vec<(String, bool)> = if matches!(self.view, View::Cheats { .. }) {
                    self.cheats
                        .iter()
                        .map(|c| (c.code.clone(), c.enabled))
                        .collect()
                } else {
                    Vec::new()
                };
                let Some(pixels) = &mut self.pixels else {
                    return;
                };
                let frame = pixels.frame_mut();
                match &self.view {
                    View::Running if nsf.is_some() => {
                        let info = nsf.as_ref().expect("guarded");
                        blit_menu(frame, |f, h| menu::render_nsf(f, h, info, nsf_secs));
                    }
                    View::Running => {
                        if let Some(nes) = &self.nes {
                            let fb = nes.framebuffer();
                            frame.copy_from_slice(&fb[skip * WIDTH * 4..(skip + h) * WIDTH * 4]);
                        }
                        if let Some(text) = &osd {
                            menu::draw_osd(frame, text);
                        }
                    }
                    View::Home { sel } => {
                        blit_menu(frame, |f, h| {
                            menu::render_home(f, h, *sel, self.nes.is_some(), self.error.as_deref())
                        });
                    }
                    View::Settings {
                        page,
                        sel,
                        waiting,
                        player,
                    } => {
                        blit_menu(frame, |f, h| {
                            menu::render_settings(f, h, &self.cfg, *page, *sel, *waiting, *player)
                        });
                    }
                    View::SlotPicker { saving, sel } => {
                        let (saving, sel) = (*saving, *sel);
                        let slots = picker_slots.expect("set whenever view is SlotPicker");
                        blit_menu(frame, |f, h| menu::render_slots(f, h, saving, sel, &slots));
                    }
                    View::Cheats { sel, entry, msg } => {
                        blit_menu(frame, |f, h| {
                            menu::render_cheats(
                                f,
                                h,
                                &cheat_rows,
                                *sel,
                                entry.as_deref(),
                                msg.as_deref(),
                            )
                        });
                    }
                }
                if let Err(e) = pixels.render() {
                    eprintln!("render error: {e}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !matches!(self.view, View::Running) || self.nes.is_none() {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        // Sample physical gamepads once per wake-up and merge with the keyboard.
        self.pads = self
            .gamepads
            .as_mut()
            .map_or([PadState::default(); MAX_PADS], |g| g.poll());
        let period = frame_period(self.nes.as_ref().unwrap());
        let now = Instant::now();
        let mut ran = false;
        // Set when a frame panics (a malformed/tampered ROM that slipped past
        // load-time checks and trips a mapper's bank math mid-run). We unload
        // the ROM and surface an error instead of letting the panic abort the
        // whole emulator.
        let mut crash: Option<String> = None;
        let mut catch_up = 0;
        'frames: while now >= self.next_frame && catch_up < 3 {
            if self.rewinding {
                if let Err(e) = self.rewind_frame() {
                    crash = Some(e);
                    break;
                }
            } else {
                // Fast-forward runs several frames per displayed frame.
                let n = if self.fast_forward {
                    self.cfg.ff_speed
                } else {
                    1
                };
                for _ in 0..n {
                    if let Err(e) = self.emulate_frame() {
                        crash = Some(e);
                        break 'frames;
                    }
                }
            }
            self.next_frame += period;
            ran = true;
            catch_up += 1;
        }
        if now >= self.next_frame {
            // fell too far behind (e.g. window drag); resync instead of spiraling
            self.next_frame = now + period;
        }
        if ran && crash.is_none() {
            let nes = self.nes.as_mut().unwrap();
            let samples = nes.take_audio();
            if let Some(audio) = &self.audio {
                if self.rewinding {
                    // Rewound audio would be garbled jumps; stay silent.
                    audio.clear();
                } else {
                    // Fast-forward: keep every Nth sample so the sped-up
                    // sound plays in real time (higher pitch) instead of
                    // piling up latency.
                    let n = if self.fast_forward {
                        self.cfg.ff_speed as usize
                    } else {
                        1
                    };
                    let kept: Vec<f32> = samples.into_iter().step_by(n).collect();
                    audio.push(&kept);
                    // dynamic rate control: nudge resampling so the queue hovers
                    // around ~50 ms instead of slowly drifting to under/overflow
                    let target = audio.sample_rate as f64 * 0.05;
                    let err = ((audio.buffered() as f64 - target) / target).clamp(-1.0, 1.0);
                    nes.tune_audio(audio.sample_rate as f64 * (1.0 - 0.003 * err));
                }
            }
            self.redraw();
        }
        if let Some(msg) = crash {
            // The machine is in an unknown state after a panic; drop it whole
            // rather than reuse it. Skip the .sav write - its RAM may be corrupt.
            eprintln!("emulation panicked, unloading ROM: {msg}");
            self.nes = None;
            self.sav_path = None;
            self.rom_path = None;
            self.rewind.clear();
            self.release_held_inputs();
            self.error = Some(format!("EMULATION CRASHED: {msg}"));
            self.view = View::Home { sel: 0 };
            self.clear_audio();
            self.apply_view_size();
            self.update_cursor();
            self.redraw();
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

/// Draw a menu into `frame`. The render fn lays the menu out for the visible
/// window height `h` (passed in), so it fills the running game's size with no
/// letterbox bars. Font drawing clamps to the full 240-line buffer, so render
/// into a full-height scratch buffer and copy the top `h` lines the layout
/// already targets.
fn blit_menu(frame: &mut [u8], draw: impl FnOnce(&mut [u8], i32)) {
    let h = frame.len() / (WIDTH * 4);
    if h >= HEIGHT {
        draw(frame, h as i32);
        return;
    }
    let mut full = vec![0u8; WIDTH * HEIGHT * 4];
    draw(&mut full, h as i32);
    frame.copy_from_slice(&full[..h * WIDTH * 4]);
}

/// Turn a caught panic payload into a printable message.
fn panic_msg(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Build a `Nes`, converting a panic in the cartridge/mapper setup (e.g. a
/// malformed header that survives `load_rom` but trips a mapper's bank math
/// at reset) into an `Err`. Untrusted ROM files must never crash the
/// emulator, only fail to load.
fn safe_build(build: impl FnOnce() -> Result<Nes, String>) -> Result<Nes, String> {
    match catch_unwind(AssertUnwindSafe(build)) {
        Ok(res) => res,
        Err(p) => Err(format!("ROM rejected: {}", panic_msg(p))),
    }
}

/// Write `data` to `path` atomically: write a sibling temp file, then rename it
/// over the target. A crash or full disk mid-write leaves the previous file
/// intact instead of a truncated one - important for .sav/.state files that
/// hold the player's progress. Falls back to a direct write if no temp path can
/// be formed.
fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Rename can fail across some filesystems; don't leave the temp behind.
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Set or clear `bit` in a button mask.
fn set_mask(mask: &mut u8, bit: u8, on: bool) {
    if on {
        *mask |= bit;
    } else {
        *mask &= !bit;
    }
}

fn main() {
    let cfg = Config::load();

    let audio = match audio::Audio::new() {
        Ok(a) => Some(a),
        Err(e) => {
            eprintln!("audio disabled: {e}");
            None
        }
    };

    // Physical gamepads are optional: keyboard play still works without them.
    let gamepads = gamepad::Gamepads::new();

    let event_loop = EventLoop::new().expect("create event loop");
    let mut app = App {
        window: None,
        pixels: None,
        nes: None,
        view: View::Home { sel: 0 },
        cfg,
        error: None,
        next_frame: Instant::now(),
        audio,
        sav_path: None,
        gamepads,
        rom_path: None,
        kb_mask: [0; 4],
        kb_turbo: [0; 4],
        pads: [PadState::default(); MAX_PADS],
        last_click: None,
        frame_count: 0,
        fast_forward: false,
        rewinding: false,
        rewind: Rewind::new(REWIND_CAPACITY),
        muted: false,
        cheats: Vec::new(),
        cursor: None,
        zapper_trigger: false,
        zapper_offscreen: false,
        osd: None,
        fds_side: 0,
        track_start: 0,
    };
    app.apply_volume();
    // optional CLI arg still works: jump straight into a ROM
    if let Some(path) = std::env::args().nth(1) {
        app.open_rom(Path::new(&path));
    }
    event_loop.run_app(&mut app).expect("event loop");
    // single exit point for quit/close/escape-from-home
    app.save_battery_ram();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cheat_file_roundtrip() {
        let list = parse_cheat_file("# header\nsxiopo\n-075A:09\n\nnot a code\n  -YEUZUGAA \n");
        let got: Vec<(&str, bool)> = list.iter().map(|c| (c.code.as_str(), c.enabled)).collect();
        assert_eq!(
            got,
            vec![("SXIOPO", true), ("075A:09", false), ("YEUZUGAA", false)]
        );
        let again = parse_cheat_file(&format_cheat_file(&list));
        assert_eq!(again.len(), 3);
        assert!(!again[1].enabled);
    }
}
