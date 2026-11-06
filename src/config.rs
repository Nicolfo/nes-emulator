//! Persistent settings: button mapping, window scale, audio volume, speed
//! and input-device options. Saved as JSON in the
//! platform's per-user config directory so settings persist no matter the
//! working directory (e.g. when launched from Finder or a desktop launcher,
//! where the cwd is `/` or `$HOME` rather than the binary's folder).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use nes_emulator::cartridge::Region;
use nes_emulator::controller::{
    BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SELECT, BTN_START, BTN_UP, InputDevice,
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
            && std::path::Path::new(&xdg).is_absolute()
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

/// Rebindable inputs per player: the 8 pad buttons then turbo A / turbo B.
pub const NUM_BINDINGS: usize = 10;
/// File names tried for the Famicom Disk System BIOS.
const FDS_BIOS_NAMES: [&str; 2] = ["disksys.rom", "DISKSYS.ROM"];

/// Locate and read the 8KB FDS BIOS. Looked for next to the disk image,
/// then in the per-user config directory, then next to the executable.
/// Returns the bytes, or the list of places searched (for the error).
pub fn find_fds_bios(disk_path: &std::path::Path) -> Result<Vec<u8>, Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = disk_path.parent() {
        dirs.push(d.to_path_buf());
    }
    if let Some(d) = config_dir() {
        dirs.push(d);
    }
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
    {
        dirs.push(d);
    }
    let mut tried = Vec::new();
    for dir in dirs {
        for name in FDS_BIOS_NAMES {
            let p = dir.join(name);
            if let Ok(data) = std::fs::read(&p)
                && data.len() == 0x2000
            {
                return Ok(data);
            }
            tried.push(p);
        }
    }
    Err(tried)
}

pub const BUTTON_LABELS: [&str; NUM_BINDINGS] = [
    "A", "B", "SELECT", "START", "UP", "DOWN", "LEFT", "RIGHT", "TURBO A", "TURBO B",
];
/// NES button bit driven by each binding (turbo rows pulse A / B).
pub const BUTTON_MASKS: [u8; NUM_BINDINGS] = [
    BTN_A, BTN_B, BTN_SELECT, BTN_START, BTN_UP, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_A, BTN_B,
];
/// Players with keyboard bindings (3 and 4 need a Four Score).
pub const PLAYERS: usize = 4;

/// Fixed hotkeys handled before button bindings while a game runs; binding
/// a button to one of them would never fire.
pub const RESERVED_KEYS: [KeyCode; 9] = [
    KeyCode::Escape,
    KeyCode::F3,
    KeyCode::F5,
    KeyCode::F6,
    KeyCode::F7,
    KeyCode::F8,
    KeyCode::F11,
    KeyCode::Tab,
    KeyCode::Backspace,
];

pub fn is_reserved(code: KeyCode) -> bool {
    RESERVED_KEYS.contains(&code)
}

/// Placeholder keys for bindings displaced on load (rarely on a keyboard,
/// so they act as "unbound" until the user picks a real key).
const SPARE_KEYS: [KeyCode; 12] = [
    KeyCode::F13,
    KeyCode::F14,
    KeyCode::F15,
    KeyCode::F16,
    KeyCode::F17,
    KeyCode::F18,
    KeyCode::F19,
    KeyCode::F20,
    KeyCode::F21,
    KeyCode::F22,
    KeyCode::F23,
    KeyCode::F24,
];

/// Bindings at or past this index are turbo (auto-fire) buttons.
pub const FIRST_TURBO: usize = 8;

pub const MIN_FF_SPEED: u8 = 2;
pub const MAX_FF_SPEED: u8 = 8;

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
    /// Turbo A / turbo B keys per player (auto-fire while held).
    #[serde(default = "default_turbo_p1")]
    pub turbo_keys: [KeyCode; 2],
    #[serde(default = "default_turbo_p2")]
    pub turbo_keys_p2: [KeyCode; 2],
    /// Players 3 and 4 (reach the game through a Four Score).
    #[serde(default = "default_keys_p3")]
    pub keys_p3: [KeyCode; 8],
    #[serde(default = "default_turbo_p3")]
    pub turbo_keys_p3: [KeyCode; 2],
    #[serde(default = "default_keys_p4")]
    pub keys_p4: [KeyCode; 8],
    #[serde(default = "default_turbo_p4")]
    pub turbo_keys_p4: [KeyCode; 2],
    /// Master volume, 0-100 in steps of 10.
    #[serde(default = "default_volume")]
    pub volume: u8,
    /// Emulation speed multiplier while the fast-forward key is held.
    #[serde(default = "default_ff_speed")]
    pub ff_speed: u8,
    /// What is plugged into the controller ports.
    #[serde(default)]
    pub input_device: InputDevice,
    /// Keep a rewind history (hold Backspace to rewind).
    #[serde(default = "default_true")]
    pub rewind: bool,
    /// TV system: follow the ROM header, or force one.
    #[serde(default)]
    pub region: RegionPref,
}

/// Region setting: the ROM header's choice or a forced TV system.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RegionPref {
    #[default]
    Auto,
    Ntsc,
    Pal,
    Dendy,
}

impl RegionPref {
    const ALL: [RegionPref; 4] = [
        RegionPref::Auto,
        RegionPref::Ntsc,
        RegionPref::Pal,
        RegionPref::Dendy,
    ];

    /// The forced region, None for Auto.
    pub fn forced(self) -> Option<Region> {
        match self {
            RegionPref::Auto => None,
            RegionPref::Ntsc => Some(Region::Ntsc),
            RegionPref::Pal => Some(Region::Pal),
            RegionPref::Dendy => Some(Region::Dendy),
        }
    }

    pub fn step(self, delta: i32) -> Self {
        let i = Self::ALL.iter().position(|&r| r == self).unwrap_or(0) as i32;
        Self::ALL[(i + delta).rem_euclid(Self::ALL.len() as i32) as usize]
    }

    pub fn label(self) -> &'static str {
        match self {
            RegionPref::Auto => "AUTO",
            RegionPref::Ntsc => "NTSC",
            RegionPref::Pal => "PAL",
            RegionPref::Dendy => "DENDY",
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_volume() -> u8 {
    100
}

fn default_ff_speed() -> u8 {
    4
}

fn default_turbo_p1() -> [KeyCode; 2] {
    [KeyCode::KeyC, KeyCode::KeyV]
}

fn default_turbo_p2() -> [KeyCode; 2] {
    [KeyCode::KeyO, KeyCode::KeyI]
}

// Player 2 defaults: left-hand WASD d-pad + right-hand action cluster, chosen
// to avoid colliding with the player 1 bindings.
/// Player 3 defaults: the numeric keypad.
fn default_keys_p3() -> [KeyCode; 8] {
    [
        KeyCode::Numpad3, // A
        KeyCode::Numpad1, // B
        KeyCode::Numpad7, // SELECT
        KeyCode::Numpad9, // START
        KeyCode::Numpad8, // UP
        KeyCode::Numpad5, // DOWN
        KeyCode::Numpad4, // LEFT
        KeyCode::Numpad6, // RIGHT
    ]
}

fn default_turbo_p3() -> [KeyCode; 2] {
    [KeyCode::NumpadAdd, KeyCode::NumpadSubtract]
}

/// Player 4 defaults: a TFGH cluster in the middle of the keyboard.
fn default_keys_p4() -> [KeyCode; 8] {
    [
        KeyCode::KeyY,   // A
        KeyCode::KeyR,   // B
        KeyCode::Digit5, // SELECT
        KeyCode::Digit6, // START
        KeyCode::KeyT,   // UP
        KeyCode::KeyG,   // DOWN
        KeyCode::KeyF,   // LEFT
        KeyCode::KeyH,   // RIGHT
    ]
}

fn default_turbo_p4() -> [KeyCode; 2] {
    [KeyCode::KeyU, KeyCode::KeyE]
}

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
            turbo_keys: default_turbo_p1(),
            turbo_keys_p2: default_turbo_p2(),
            keys_p3: default_keys_p3(),
            turbo_keys_p3: default_turbo_p3(),
            keys_p4: default_keys_p4(),
            turbo_keys_p4: default_turbo_p4(),
            volume: default_volume(),
            ff_speed: default_ff_speed(),
            input_device: InputDevice::Standard,
            rewind: true,
            region: RegionPref::Auto,
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
            .and_then(|s| Self::from_json(&s))
            .unwrap_or_default()
    }

    /// Parse a saved config, clamping values and repairing key bindings
    /// (see [`Config::sanitize_bindings`]).
    fn from_json(s: &str) -> Option<Self> {
        let raw: serde_json::Value = serde_json::from_str(s).ok()?;
        let mut c: Config = serde_json::from_value(raw.clone()).ok()?;
        c.scale = c.scale.clamp(1, 5);
        c.volume = c.volume.min(100) / 10 * 10;
        c.ff_speed = c.ff_speed.clamp(MIN_FF_SPEED, MAX_FF_SPEED);
        c.sanitize_bindings(|field| raw.get(field).is_some());
        Some(c)
    }

    /// Repair bindings after loading. Fields missing from the file got
    /// their defaults, which may clash with keys the user already bound
    /// (e.g. an old config with B on C, now also the default turbo A); and
    /// a binding to a reserved hotkey can never fire. Saved bindings claim
    /// their keys first; any slot left clashing or reserved moves to a
    /// spare key (F13-F24) the user can rebind in Settings.
    fn sanitize_bindings(&mut self, saved: impl Fn(&str) -> bool) {
        const FIELDS: [(&str, &str); PLAYERS] = [
            ("keys", "turbo_keys"),
            ("keys_p2", "turbo_keys_p2"),
            ("keys_p3", "turbo_keys_p3"),
            ("keys_p4", "turbo_keys_p4"),
        ];
        let slots: Vec<(usize, usize, bool)> = (0..PLAYERS)
            .flat_map(|p| (0..NUM_BINDINGS).map(move |b| (p, b)))
            .map(|(p, b)| {
                let field = if b < FIRST_TURBO {
                    FIELDS[p].0
                } else {
                    FIELDS[p].1
                };
                (p, b, saved(field))
            })
            .collect();
        let mut used: Vec<KeyCode> = Vec::new();
        let mut needs_spare = Vec::new();
        // Saved slots first, then defaulted ones.
        for pass in [true, false] {
            for &(p, b, from_file) in slots.iter().filter(|s| s.2 == pass) {
                let k = self.binding(p, b);
                let clash = !from_file && used.contains(&k);
                if is_reserved(k) || clash {
                    needs_spare.push((p, b));
                } else {
                    used.push(k);
                }
            }
        }
        let mut spares = SPARE_KEYS.iter().filter(|k| !used.contains(k));
        for (p, b) in needs_spare {
            if let Some(&k) = spares.next() {
                self.set_binding(p, b, k);
            }
        }
    }

    fn set_binding(&mut self, player: usize, binding: usize, code: KeyCode) {
        let (keys, turbo) = match player {
            0 => (&mut self.keys, &mut self.turbo_keys),
            1 => (&mut self.keys_p2, &mut self.turbo_keys_p2),
            2 => (&mut self.keys_p3, &mut self.turbo_keys_p3),
            _ => (&mut self.keys_p4, &mut self.turbo_keys_p4),
        };
        if binding < FIRST_TURBO {
            keys[binding] = code;
        } else {
            turbo[binding - FIRST_TURBO] = code;
        }
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            // Write to a temp sibling then rename over the target, so an
            // interrupted write can't leave a truncated (unparseable) config
            // that would reset all the user's bindings on next launch.
            let path = config_path();
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    /// Key bound to `binding` (index into [`BUTTON_LABELS`]) for `player`.
    pub fn binding(&self, player: usize, binding: usize) -> KeyCode {
        let (keys, turbo) = self.player_keys(player);
        if binding < FIRST_TURBO {
            keys[binding]
        } else {
            turbo[binding - FIRST_TURBO]
        }
    }

    fn player_keys(&self, player: usize) -> (&[KeyCode; 8], &[KeyCode; 2]) {
        match player {
            0 => (&self.keys, &self.turbo_keys),
            1 => (&self.keys_p2, &self.turbo_keys_p2),
            2 => (&self.keys_p3, &self.turbo_keys_p3),
            _ => (&self.keys_p4, &self.turbo_keys_p4),
        }
    }

    /// Bind `code` to `binding` for `player`. A key already bound to another
    /// of that player's inputs is swapped onto the old key so every input
    /// stays reachable.
    pub fn rebind(&mut self, player: usize, binding: usize, code: KeyCode) {
        let old = self.binding(player, binding);
        let (keys, turbo) = match player {
            0 => (&mut self.keys, &mut self.turbo_keys),
            1 => (&mut self.keys_p2, &mut self.turbo_keys_p2),
            2 => (&mut self.keys_p3, &mut self.turbo_keys_p3),
            _ => (&mut self.keys_p4, &mut self.turbo_keys_p4),
        };
        let mut all: Vec<&mut KeyCode> = keys.iter_mut().chain(turbo.iter_mut()).collect();
        if let Some(other) = all.iter().position(|k| **k == code) {
            *all[other] = old;
        }
        *all[binding] = code;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bindings_never_collide_across_players() {
        let c = Config::default();
        let mut all: Vec<KeyCode> = (0..PLAYERS)
            .flat_map(|p| (0..NUM_BINDINGS).map(move |b| (p, b)))
            .map(|(p, b)| c.binding(p, b))
            .collect();
        let n = all.len();
        all.sort_by_key(|k| format!("{k:?}"));
        all.dedup();
        assert_eq!(all.len(), n, "a default key is bound twice");
    }

    #[test]
    fn upgraded_config_keeps_user_keys_over_new_defaults() {
        // Old config: P1 B on C (now the default turbo A), P2 A on Tab
        // (now the fast-forward hotkey). No turbo / P3 / P4 fields.
        let json = r#"{"keys":["KeyZ","KeyC","ShiftRight","Enter","ArrowUp","ArrowDown","ArrowLeft","ArrowRight"],
            "keys_p2":["Tab","KeyK","KeyN","KeyM","KeyW","KeyS","KeyA","KeyD"],"scale":3}"#;
        let c = Config::from_json(json).unwrap();
        assert_eq!(c.binding(0, 1), KeyCode::KeyC, "user binding kept");
        assert_ne!(
            c.binding(0, FIRST_TURBO),
            KeyCode::KeyC,
            "default turbo moved"
        );
        assert!(!is_reserved(c.binding(1, 0)), "reserved key replaced");
        let mut all: Vec<String> = (0..PLAYERS)
            .flat_map(|p| (0..NUM_BINDINGS).map(move |b| (p, b)))
            .map(|(p, b)| format!("{:?}", c.binding(p, b)))
            .collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "no key bound twice after repair");
    }

    #[test]
    fn rebind_swaps_across_turbo_rows() {
        let mut c = Config::default();
        // Bind P1 turbo A to Z, which is P1's A: A must inherit turbo A's key.
        c.rebind(0, FIRST_TURBO, KeyCode::KeyZ);
        assert_eq!(c.binding(0, FIRST_TURBO), KeyCode::KeyZ);
        assert_eq!(c.binding(0, 0), KeyCode::KeyC);
        c.rebind(1, 3, KeyCode::KeyI); // P2 START <- P2 turbo B's key
        assert_eq!(c.binding(1, 3), KeyCode::KeyI);
        assert_eq!(c.binding(1, FIRST_TURBO + 1), KeyCode::KeyM);
    }

    #[test]
    fn old_configs_get_defaults_for_new_fields() {
        let json = r#"{"keys":["KeyZ","KeyX","ShiftRight","Enter","ArrowUp","ArrowDown","ArrowLeft","ArrowRight"],"scale":2}"#;
        let c: Config = serde_json::from_str(json).unwrap();
        assert_eq!(c.volume, 100);
        assert_eq!(c.ff_speed, 4);
        assert_eq!(c.input_device, InputDevice::Standard);
        assert!(c.rewind);
        assert_eq!(c.turbo_keys, default_turbo_p1());
    }
}
