//! Persistent settings: button mapping + window scale. Saved as JSON in the
//! platform's per-user config directory so settings persist no matter the
//! working directory (e.g. when launched from Finder or a desktop launcher,
//! where the cwd is `/` or `$HOME` rather than the binary's folder).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use nes_emulator::controller::{
    BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SELECT, BTN_START, BTN_UP,
};

/// Config filename inside the per-user config directory.
const CONFIG_FILE: &str = "config.json";
/// Legacy location used before per-user config dirs: a file next to the
/// working directory. Still read once for a one-time migration so existing
/// users keep their bindings, and used as a last-resort fallback when no
/// config directory is available or writable.
const LEGACY_CONFIG_PATH: &str = "nes-emulator-config.json";

/// Per-user config directory for this app, following each platform's
/// convention. `None` only if the environment defines no home/config location
/// (then we fall back to the legacy cwd-relative file).
fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library/Application Support/nes-emulator"))
    }
    #[cfg(target_os = "windows")]
    {
        // %APPDATA% (Roaming) is the conventional home for per-user app config.
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join("nes-emulator"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // XDG Base Directory spec: $XDG_CONFIG_HOME if set to an absolute path,
        // else ~/.config. The spec says relative values must be ignored - and
        // honoring one would put config back under the cwd, the very thing this
        // is meant to avoid.
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
            && Path::new(&xdg).is_absolute()
        {
            return Some(PathBuf::from(xdg).join("nes-emulator"));
        }
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join(".config/nes-emulator"))
    }
}

/// Absolute path the config is read from and written to, creating the parent
/// directory if needed. Falls back to the legacy cwd-relative filename when no
/// config directory can be resolved *or* it can't be created (e.g. a read-only
/// home), so persistence degrades gracefully instead of failing silently.
fn config_path() -> PathBuf {
    match config_dir() {
        Some(dir) if std::fs::create_dir_all(&dir).is_ok() => dir.join(CONFIG_FILE),
        _ => PathBuf::from(LEGACY_CONFIG_PATH),
    }
}

pub const BUTTON_LABELS: [&str; 8] = ["A", "B", "SELECT", "START", "UP", "DOWN", "LEFT", "RIGHT"];
pub const BUTTON_MASKS: [u8; 8] = [
    BTN_A, BTN_B, BTN_SELECT, BTN_START, BTN_UP, BTN_DOWN, BTN_LEFT, BTN_RIGHT,
];

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub keys: [KeyCode; 8], // player 1, same order as BUTTON_LABELS
    #[serde(default = "default_keys_p2")]
    pub keys_p2: [KeyCode; 8], // player 2, same order as BUTTON_LABELS
    pub scale: u32,
    // NTSC TVs hide the top/bottom 8 scanlines (overscan); games rely on it
    // to hide raster-split garbage (e.g. Castlevania III's title).
    #[serde(default = "default_true")]
    pub crop_overscan: bool,
}

fn default_true() -> bool {
    true
}

// Player 2 defaults: left-hand WASD d-pad + right-hand action cluster, chosen
// to avoid colliding with the player 1 bindings.
fn default_keys_p2() -> [KeyCode; 8] {
    [
        KeyCode::KeyL, // A
        KeyCode::KeyK, // B
        KeyCode::KeyN, // SELECT
        KeyCode::KeyM, // START
        KeyCode::KeyW, // UP
        KeyCode::KeyS, // DOWN
        KeyCode::KeyA, // LEFT
        KeyCode::KeyD, // RIGHT
    ]
}

impl Default for Config {
    fn default() -> Self {
        Config {
            keys: [
                KeyCode::KeyZ,
                KeyCode::KeyX,
                KeyCode::ShiftRight,
                KeyCode::Enter,
                KeyCode::ArrowUp,
                KeyCode::ArrowDown,
                KeyCode::ArrowLeft,
                KeyCode::ArrowRight,
            ],
            keys_p2: default_keys_p2(),
            scale: 3,
            crop_overscan: true,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        // Prefer the standard config path; if absent, fall back to the legacy
        // cwd file so a pre-existing config migrates on the next save().
        std::fs::read_to_string(config_path())
            .or_else(|_| std::fs::read_to_string(LEGACY_CONFIG_PATH))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .map(|mut c: Config| {
                c.scale = c.scale.clamp(1, 5);
                c
            })
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(config_path(), json);
        }
    }

    /// Human-readable key name, e.g. "Z", "ARROW UP", "RIGHT SHIFT".
    pub fn key_name(code: KeyCode) -> String {
        let dbg = format!("{code:?}");
        let stripped = dbg
            .strip_prefix("Key")
            .or_else(|| dbg.strip_prefix("Digit"))
            .unwrap_or(&dbg);
        // split CamelCase into words: ArrowUp -> ARROW UP
        let mut out = String::new();
        for (i, ch) in stripped.chars().enumerate() {
            if i > 0 && ch.is_uppercase() {
                out.push(' ');
            }
            out.push(ch.to_ascii_uppercase());
        }
        out
    }
}
