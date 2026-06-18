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
use winit::window::{Fullscreen, Window, WindowId};

use nes_emulator::cartridge::Region;
use nes_emulator::nes::Nes;
use nes_emulator::ppu::{HEIGHT, WIDTH};

use config::{BUTTON_MASKS, Config};
use menu::{
    HomeAction, ROW_BACK, ROW_OVERSCAN, ROW_PLAYER, ROW_RESET, ROW_SCALE, SETTINGS_ROWS, home_items,
};

/// Scanlines hidden by NTSC overscan. Top crop is deeper: raster-split
/// games (e.g. Castlevania III) finish their scanline-IRQ bank switch a
/// line or two into the frame, so garbage can extend past the classic 8.
const OVERSCAN_TOP: usize = 16;
const OVERSCAN_BOTTOM: usize = 8;

// NTSC: 89342 PPU dots per frame at 5,369,318 dots/sec = 60.0988 FPS
const FRAME_PERIOD: Duration = Duration::from_nanos(16_639_267);
// PAL: 106392 PPU dots per frame at 5,320,342.5 dots/sec = 50.0070 FPS
const PAL_FRAME_PERIOD: Duration = Duration::from_nanos(19_997_200);

fn frame_period(nes: &Nes) -> Duration {
    match nes.region() {
        Region::Ntsc => FRAME_PERIOD,
        Region::Pal => PAL_FRAME_PERIOD,
    }
}

enum View {
    Home {
        sel: usize,
    },
    Settings {
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
    Running,
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
    /// Per-player button masks from the keyboard and the gamepad, kept apart
    /// so one input source never clears buttons the other is holding. The
    /// controller sees their union (see `apply_inputs`).
    kb_mask: [u8; 2],
    pad_mask: [u8; 2],
    /// Timestamp of the last left-button press, for double-click detection.
    last_click: Option<Instant>,
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

    /// Push the merged keyboard+gamepad button state into both controllers.
    /// The two sources are OR-ed so neither clears the other's held buttons.
    fn apply_inputs(&mut self) {
        if let Some(nes) = &mut self.nes {
            nes.cpu.bus.controller1.state = self.kb_mask[0] | self.pad_mask[0];
            nes.cpu.bus.controller2.state = self.kb_mask[1] | self.pad_mask[1];
        }
    }

    /// Leave a menu or the slot picker and resume play. Resetting next_frame
    /// stops the catch-up loop from fast-forwarding over the paused time.
    fn resume_game(&mut self) {
        self.view = View::Running;
        self.next_frame = Instant::now();
        self.redraw();
    }

    /// Path of savestate `slot` (0-based): `<rom>.stateN`, next to the ROM.
    fn state_path(&self, slot: usize) -> Option<PathBuf> {
        self.sav_path
            .as_ref()
            .map(|p| p.with_extension(format!("state{}", slot + 1)))
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
            Ok(()) => eprintln!("saved state to {}", path.display()),
            Err(e) => eprintln!("save state failed: {e}"),
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
                self.clear_audio();
                self.redraw();
            }
            Err(e) => eprintln!("load state failed: {e}"),
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

    fn load_rom_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("NES ROM", &["nes"])
            .set_title("Load NES ROM")
            .pick_file();
        let Some(path) = picked else { return };
        match std::fs::read(&path) {
            Ok(rom) => match safe_new(&rom) {
                Ok(mut nes) => {
                    self.save_battery_ram(); // flush the outgoing game first
                    if let Some(a) = &self.audio {
                        nes.set_sample_rate(a.sample_rate as f64);
                    }
                    self.sav_path = Some(restore_battery_ram(&mut nes, &path));
                    self.nes = Some(nes);
                    self.error = None;
                    if let Some(w) = &self.window {
                        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("NES");
                        w.set_title(&format!("NES Emulator - {name}"));
                    }
                    self.start_running();
                }
                Err(e) => self.error = Some(e),
            },
            Err(e) => self.error = Some(format!("READ FAILED: {e}")),
        }
    }

    fn start_running(&mut self) {
        self.view = View::Running;
        self.next_frame = Instant::now();
        self.apply_view_size();
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
                    HomeAction::Settings => {
                        self.view = View::Settings {
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
            sel,
            waiting,
            player,
        } = &mut self.view
        else {
            return;
        };

        if *waiting {
            // capture next key as the new binding (Escape cancels)
            if code != KeyCode::Escape {
                let row = *sel;
                let keys = if *player == 0 {
                    &mut self.cfg.keys
                } else {
                    &mut self.cfg.keys_p2
                };
                let old = keys[row];
                // if the key is already bound elsewhere (same player), swap to
                // keep all buttons usable
                if let Some(other) = keys.iter().position(|&k| k == code) {
                    keys[other] = old;
                }
                keys[row] = code;
                self.cfg.save();
            }
            *waiting = false;
            self.redraw();
            return;
        }

        match code {
            KeyCode::ArrowUp => *sel = (*sel + SETTINGS_ROWS - 1) % SETTINGS_ROWS,
            KeyCode::ArrowDown => *sel = (*sel + 1) % SETTINGS_ROWS,
            KeyCode::ArrowLeft | KeyCode::ArrowRight if *sel == ROW_PLAYER => {
                *player ^= 1;
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight if *sel == ROW_SCALE => {
                let delta = if code == KeyCode::ArrowLeft { -1i32 } else { 1 };
                self.cfg.scale = (self.cfg.scale as i32 + delta).clamp(1, 5) as u32;
                self.cfg.save();
                self.apply_view_size();
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight if *sel == ROW_OVERSCAN => {
                self.cfg.crop_overscan = !self.cfg.crop_overscan;
                self.cfg.save();
            }
            KeyCode::Enter | KeyCode::Space => match *sel {
                0..=7 => *waiting = true,
                ROW_PLAYER => *player ^= 1,
                ROW_SCALE => {
                    self.cfg.scale = self.cfg.scale % 5 + 1;
                    self.cfg.save();
                    self.apply_view_size();
                }
                ROW_OVERSCAN => {
                    self.cfg.crop_overscan = !self.cfg.crop_overscan;
                    self.cfg.save();
                }
                ROW_RESET => {
                    let scale = self.cfg.scale;
                    self.cfg = Config {
                        scale,
                        ..Config::default()
                    };
                    self.cfg.save();
                }
                ROW_BACK => self.view = View::Home { sel: 0 },
                _ => {}
            },
            KeyCode::Escape => self.view = View::Home { sel: 0 },
            _ => {}
        }
        self.redraw();
    }

    fn running_key(&mut self, code: KeyCode, pressed: bool, repeat: bool) {
        if repeat {
            return;
        }
        if code == KeyCode::Escape && pressed {
            self.save_battery_ram();
            self.kb_mask = [0, 0];
            self.view = View::Home { sel: 0 };
            self.apply_view_size();
            self.clear_audio();
            // Make sure the pause menu comes to the front and takes input.
            if let Some(w) = &self.window {
                w.focus_window();
            }
            self.redraw();
            return;
        }
        // Savestate slots: F5 opens the save picker, F7 the load picker.
        // The picker pauses the game (about_to_wait only ticks in Running).
        if pressed && (code == KeyCode::F5 || code == KeyCode::F7) {
            if self.nes.is_some() {
                self.view = View::SlotPicker {
                    saving: code == KeyCode::F5,
                    sel: 0,
                };
                self.kb_mask = [0, 0];
                self.clear_audio();
                self.redraw();
            }
            return;
        }
        // F3 is the console RESET button: re-enter the cartridge's reset vector
        // with RAM/VRAM intact. Not rebindable (like F5/F7).
        if pressed && code == KeyCode::F3 {
            if let Some(nes) = &mut self.nes {
                nes.reset();
            }
            return;
        }
        // The keyboard owns the keyboard half of each player's button mask; the
        // gamepad half is refreshed every frame in about_to_wait. apply_inputs
        // OR-s the two so neither source clears the other's held buttons.
        for (i, &k) in self.cfg.keys.iter().enumerate() {
            if k == code {
                set_mask(&mut self.kb_mask[0], BUTTON_MASKS[i], pressed);
            }
        }
        for (i, &k) in self.cfg.keys_p2.iter().enumerate() {
            if k == code {
                set_mask(&mut self.kb_mask[1], BUTTON_MASKS[i], pressed);
            }
        }
        self.apply_inputs();
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
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.handle_click(),
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
                let Some(pixels) = &mut self.pixels else {
                    return;
                };
                let frame = pixels.frame_mut();
                match &self.view {
                    View::Running => {
                        if let Some(nes) = &self.nes {
                            let fb = nes.framebuffer();
                            frame.copy_from_slice(&fb[skip * WIDTH * 4..(skip + h) * WIDTH * 4]);
                        }
                    }
                    View::Home { sel } => {
                        blit_menu(frame, |f, h| {
                            menu::render_home(f, h, *sel, self.nes.is_some(), self.error.as_deref())
                        });
                    }
                    View::Settings {
                        sel,
                        waiting,
                        player,
                    } => {
                        blit_menu(frame, |f, h| {
                            menu::render_settings(f, h, &self.cfg, *sel, *waiting, *player)
                        });
                    }
                    View::SlotPicker { saving, sel } => {
                        let (saving, sel) = (*saving, *sel);
                        let slots = picker_slots.expect("set whenever view is SlotPicker");
                        blit_menu(frame, |f, h| menu::render_slots(f, h, saving, sel, &slots));
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
        self.pad_mask = self.gamepads.as_mut().map_or([0, 0], |g| g.poll());
        self.apply_inputs();
        let period = frame_period(self.nes.as_ref().unwrap());
        let now = Instant::now();
        let mut ran = false;
        // Set when a frame panics (a malformed/tampered ROM that slipped past
        // load-time checks and trips a mapper's bank math mid-run). We unload
        // the ROM and surface an error instead of letting the panic abort the
        // whole emulator.
        let mut crash: Option<String> = None;
        let nes = self.nes.as_mut().unwrap();
        let mut catch_up = 0;
        while now >= self.next_frame && catch_up < 3 {
            if let Err(p) = catch_unwind(AssertUnwindSafe(|| nes.run_frame())) {
                crash = Some(panic_msg(p));
                break;
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
            let samples = nes.take_audio();
            if let Some(audio) = &self.audio {
                audio.push(&samples);
                // dynamic rate control: nudge resampling so the queue hovers
                // around ~50 ms instead of slowly drifting to under/overflow
                let target = audio.sample_rate as f64 * 0.05;
                let err = ((audio.buffered() as f64 - target) / target).clamp(-1.0, 1.0);
                nes.tune_audio(audio.sample_rate as f64 * (1.0 - 0.003 * err));
            }
            self.redraw();
        }
        if let Some(msg) = crash {
            // The machine is in an unknown state after a panic; drop it whole
            // rather than reuse it. Skip the .sav write - its RAM may be corrupt.
            eprintln!("emulation panicked, unloading ROM: {msg}");
            self.nes = None;
            self.sav_path = None;
            self.error = Some(format!("EMULATION CRASHED: {msg}"));
            self.view = View::Home { sel: 0 };
            self.clear_audio();
            self.apply_view_size();
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

/// Build a `Nes` from ROM bytes, converting a panic in the cartridge/mapper
/// setup (e.g. a malformed header that survives `load_rom` but trips a mapper's
/// bank math at reset) into an `Err`. Untrusted ROM files must never crash the
/// emulator, only fail to load.
fn safe_new(rom: &[u8]) -> Result<Nes, String> {
    match catch_unwind(|| Nes::new(rom)) {
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

    // optional CLI arg still works: jump straight into a ROM
    let mut nes = None;
    let mut view = View::Home { sel: 0 };
    let mut error = None;
    let mut sav_path = None;
    if let Some(path) = std::env::args().nth(1) {
        match std::fs::read(&path) {
            Ok(rom) => match safe_new(&rom) {
                Ok(mut n) => {
                    if let Some(a) = &audio {
                        n.set_sample_rate(a.sample_rate as f64);
                    }
                    sav_path = Some(restore_battery_ram(&mut n, Path::new(&path)));
                    nes = Some(n);
                    view = View::Running;
                }
                Err(e) => error = Some(e),
            },
            Err(e) => error = Some(format!("READ FAILED: {e}")),
        }
    }

    let event_loop = EventLoop::new().expect("create event loop");
    let mut app = App {
        window: None,
        pixels: None,
        nes,
        view,
        cfg,
        error,
        next_frame: Instant::now(),
        audio,
        sav_path,
        gamepads,
        kb_mask: [0, 0],
        pad_mask: [0, 0],
        last_click: None,
    };
    event_loop.run_app(&mut app).expect("event loop");
    // single exit point for quit/close/escape-from-home
    app.save_battery_ram();
}
