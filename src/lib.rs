/// `eprintln!` to stderr only when built with the `trace` feature *and* the
/// named environment variable is set. Without the feature the whole call
/// (including the env lookup and argument evaluation) is stripped at compile
/// time, so debug instrumentation costs nothing in release builds.
#[macro_export]
macro_rules! trace_log {
    ($var:expr, $($arg:tt)*) => {{
        #[cfg(feature = "trace")]
        if std::env::var($var).is_ok() {
            eprintln!($($arg)*);
        }
    }};
}

pub mod apu;
pub mod bus;
pub mod cartridge;
pub mod controller;
pub mod cpu;
pub mod mapper;
pub mod nes;
pub mod palette;
pub mod ppu;
