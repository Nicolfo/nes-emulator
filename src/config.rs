//! Persistent settings: button mapping + window scale. Saved as JSON next to the cwd.

use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use nes_emulator::controller::{
    BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SELECT, BTN_START, BTN_UP,
};

pub const CONFIG_PATH: &str = "nes-emulator-config.json";

pub const BUTTON_LABELS: [&str; 8] = ["A", "B", "SELECT", "START", "UP", "DOWN", "LEFT", "RIGHT"];
pub const BUTTON_MASKS: [u8; 8] =
    [BTN_A, BTN_B, BTN_SELECT, BTN_START, BTN_UP, BTN_DOWN, BTN_LEFT, BTN_RIGHT];

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub keys: [KeyCode; 8], // same order as BUTTON_LABELS
    pub scale: u32,
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
            scale: 3,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(CONFIG_PATH)
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
            let _ = std::fs::write(CONFIG_PATH, json);
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
