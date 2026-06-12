mod audio;
mod config;
mod font;
mod icon;
mod menu;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pixels::{Pixels, SurfaceTexture};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use nes_emulator::cartridge::Region;
use nes_emulator::nes::Nes;
use nes_emulator::ppu::{HEIGHT, WIDTH};

use config::{BUTTON_MASKS, Config};
use menu::{HomeAction, ROW_BACK, ROW_OVERSCAN, ROW_RESET, ROW_SCALE, SETTINGS_ROWS, home_items};

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
    Home { sel: usize },
    Settings { sel: usize, waiting: bool },
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
}

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

    /// Write battery RAM to the .sav file; no-op without battery/ROM.
    fn save_battery_ram(&self) {
        let (Some(nes), Some(path)) = (&self.nes, &self.sav_path) else {
            return;
        };
        if let Some(ram) = nes.battery_ram()
            && let Err(e) = std::fs::write(path, ram)
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
            Ok(rom) => match Nes::new(&rom) {
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
        if matches!(self.view, View::Running)
            && self.cfg.crop_overscan
            && self
                .nes
                .as_ref()
                .is_some_and(|n| n.region() == Region::Ntsc)
        {
            (OVERSCAN_TOP, HEIGHT - OVERSCAN_TOP - OVERSCAN_BOTTOM)
        } else {
            (0, HEIGHT)
        }
    }

    /// Match pixel buffer and window size to the current view's crop.
    fn apply_view_size(&mut self) {
        let (_, h) = self.crop();
        if let Some(p) = &mut self.pixels {
            let _ = p.resize_buffer(WIDTH as u32, h as u32);
        }
        if let Some(w) = &self.window {
            let _ = w.request_inner_size(LogicalSize::new(
                (WIDTH as u32 * self.cfg.scale) as f64,
                (h as u32 * self.cfg.scale) as f64,
            ));
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
                        }
                    }
                    HomeAction::Quit => event_loop.exit(),
                }
            }
            KeyCode::Escape => event_loop.exit(),
            _ => {}
        }
        self.redraw();
    }

    fn settings_key(&mut self, code: KeyCode) {
        let View::Settings { sel, waiting } = &mut self.view else {
            return;
        };

        if *waiting {
            // capture next key as the new binding (Escape cancels)
            if code != KeyCode::Escape {
                let row = *sel;
                let old = self.cfg.keys[row];
                // if the key is already bound elsewhere, swap to keep all buttons usable
                if let Some(other) = self.cfg.keys.iter().position(|&k| k == code) {
                    self.cfg.keys[other] = old;
                }
                self.cfg.keys[row] = code;
                self.cfg.save();
            }
            *waiting = false;
            self.redraw();
            return;
        }

        match code {
            KeyCode::ArrowUp => *sel = (*sel + SETTINGS_ROWS - 1) % SETTINGS_ROWS,
            KeyCode::ArrowDown => *sel = (*sel + 1) % SETTINGS_ROWS,
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
            self.view = View::Home { sel: 0 };
            self.apply_view_size();
            if let Some(a) = &self.audio {
                a.clear();
            }
            self.redraw();
            return;
        }
        let Some(nes) = &mut self.nes else { return };
        for (i, &k) in self.cfg.keys.iter().enumerate() {
            if k == code {
                nes.cpu.bus.controller1.set_button(BUTTON_MASKS[i], pressed);
            }
        }
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
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state.is_pressed();
                match self.view {
                    View::Running => self.running_key(code, pressed, event.repeat),
                    View::Home { .. } if pressed => self.home_key(code, event_loop),
                    View::Settings { .. } if pressed => self.settings_key(code),
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                let (skip, h) = self.crop();
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
                        menu::render_home(frame, *sel, self.nes.is_some(), self.error.as_deref());
                    }
                    View::Settings { sel, waiting } => {
                        menu::render_settings(frame, &self.cfg, *sel, *waiting);
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
        let nes = self.nes.as_mut().unwrap();
        let period = frame_period(nes);
        let now = Instant::now();
        let mut ran = false;
        let mut catch_up = 0;
        while now >= self.next_frame && catch_up < 3 {
            nes.run_frame();
            self.next_frame += period;
            ran = true;
            catch_up += 1;
        }
        if now >= self.next_frame {
            // fell too far behind (e.g. window drag); resync instead of spiraling
            self.next_frame = now + period;
        }
        if ran {
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
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
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

    // optional CLI arg still works: jump straight into a ROM
    let mut nes = None;
    let mut view = View::Home { sel: 0 };
    let mut error = None;
    let mut sav_path = None;
    if let Some(path) = std::env::args().nth(1) {
        match std::fs::read(&path) {
            Ok(rom) => match Nes::new(&rom) {
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
    };
    event_loop.run_app(&mut app).expect("event loop");
    // single exit point for quit/close/escape-from-home
    app.save_battery_ram();
}
