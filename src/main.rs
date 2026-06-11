use std::sync::Arc;
use std::time::{Duration, Instant};

use pixels::{Pixels, SurfaceTexture};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use nes_emulator::controller::{
    BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SELECT, BTN_START, BTN_UP,
};
use nes_emulator::nes::Nes;
use nes_emulator::ppu::{HEIGHT, WIDTH};

// NTSC: 89342 PPU dots per frame at 5,369,318 dots/sec = 60.0988 FPS
const FRAME_PERIOD: Duration = Duration::from_nanos(16_639_267);

struct App {
    window: Option<Arc<Window>>,
    pixels: Option<Pixels<'static>>,
    nes: Nes,
    next_frame: Instant,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("NES Emulator")
            .with_inner_size(LogicalSize::new((WIDTH * 3) as f64, (HEIGHT * 3) as f64));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let size = window.inner_size();
        let surface = SurfaceTexture::new(size.width, size.height, window.clone());
        let pixels =
            Pixels::new(WIDTH as u32, HEIGHT as u32, surface).expect("create pixel buffer");
        self.window = Some(window);
        self.pixels = Some(pixels);
        self.next_frame = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(pixels) = &mut self.pixels {
                    let _ = pixels.resize_surface(size.width.max(1), size.height.max(1));
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.repeat {
                    return;
                }
                let pressed = event.state.is_pressed();
                let pad = &mut self.nes.cpu.bus.controller1;
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) if pressed => event_loop.exit(),
                    PhysicalKey::Code(KeyCode::ArrowUp) => pad.set_button(BTN_UP, pressed),
                    PhysicalKey::Code(KeyCode::ArrowDown) => pad.set_button(BTN_DOWN, pressed),
                    PhysicalKey::Code(KeyCode::ArrowLeft) => pad.set_button(BTN_LEFT, pressed),
                    PhysicalKey::Code(KeyCode::ArrowRight) => pad.set_button(BTN_RIGHT, pressed),
                    PhysicalKey::Code(KeyCode::KeyZ) => pad.set_button(BTN_A, pressed),
                    PhysicalKey::Code(KeyCode::KeyX) => pad.set_button(BTN_B, pressed),
                    PhysicalKey::Code(KeyCode::Enter) => pad.set_button(BTN_START, pressed),
                    PhysicalKey::Code(KeyCode::ShiftRight) | PhysicalKey::Code(KeyCode::Tab) => {
                        pad.set_button(BTN_SELECT, pressed)
                    }
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(pixels) = &mut self.pixels {
                    pixels.frame_mut().copy_from_slice(self.nes.framebuffer());
                    if let Err(e) = pixels.render() {
                        eprintln!("render error: {e}");
                        event_loop.exit();
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let mut ran = false;
        let mut catch_up = 0;
        while now >= self.next_frame && catch_up < 3 {
            self.nes.run_frame();
            self.next_frame += FRAME_PERIOD;
            ran = true;
            catch_up += 1;
        }
        if now >= self.next_frame {
            // fell too far behind (e.g. window drag); resync instead of spiraling
            self.next_frame = now + FRAME_PERIOD;
        }
        if ran && let Some(w) = &self.window {
            w.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Super Mario Bros. (Japan, USA).nes".to_string());
    let rom = match std::fs::read(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("failed to read ROM '{path}': {e}");
            std::process::exit(1);
        }
    };
    let nes = match Nes::new(&rom) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("failed to load ROM '{path}': {e}");
            std::process::exit(1);
        }
    };

    let event_loop = EventLoop::new().expect("create event loop");
    let mut app = App { window: None, pixels: None, nes, next_frame: Instant::now() };
    event_loop.run_app(&mut app).expect("event loop");
}
