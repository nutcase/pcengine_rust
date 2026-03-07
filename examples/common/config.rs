use sdl2::keyboard::Keycode;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    pub window_scale: u32,
    pub panel_width: u32,
    pub input: InputBindings,
    pub performance: PerformanceConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputBindings {
    pub up: String,
    pub down: String,
    pub left: String,
    pub right: String,
    pub button_i: String,
    pub button_ii: String,
    pub rapid_i: String,
    pub rapid_ii: String,
    pub select: String,
    pub run: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PerformanceConfig {
    pub audio_batch: usize,
    pub emu_audio_batch: usize,
    pub audio_queue_min: usize,
    pub audio_queue_target: usize,
    pub audio_queue_max: usize,
    pub audio_queue_critical: usize,
    pub max_emu_steps_per_pump: usize,
    pub max_steps_after_frame: usize,
    pub max_present_interval_ms: u64,
    pub auto_fire_hz: u128,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            window_scale: 3,
            panel_width: 420,
            input: InputBindings::default(),
            performance: PerformanceConfig::default(),
        }
    }
}

impl Default for InputBindings {
    fn default() -> Self {
        Self {
            up: "Up".to_string(),
            down: "Down".to_string(),
            left: "Left".to_string(),
            right: "Right".to_string(),
            button_i: "Z".to_string(),
            button_ii: "X".to_string(),
            rapid_i: "A".to_string(),
            rapid_ii: "S".to_string(),
            select: "LShift".to_string(),
            run: "Return".to_string(),
        }
    }
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            audio_batch: 512,
            emu_audio_batch: 128,
            audio_queue_min: 512 * 2,
            audio_queue_target: 512 * 4,
            audio_queue_max: 512 * 6,
            audio_queue_critical: 512,
            max_emu_steps_per_pump: 120_000,
            max_steps_after_frame: 30_000,
            max_present_interval_ms: 33,
            auto_fire_hz: 22,
        }
    }
}

impl AppConfig {
    pub fn load(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_else(|_| Self::default()),
            Err(_) => Self::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ParsedBindings {
    pub up: Keycode,
    pub down: Keycode,
    pub left: Keycode,
    pub right: Keycode,
    pub button_i: Keycode,
    pub button_ii: Keycode,
    pub rapid_i: Keycode,
    pub rapid_ii: Keycode,
    pub select: Keycode,
    pub run: Keycode,
}

impl ParsedBindings {
    pub fn from_input(input: &InputBindings) -> Option<Self> {
        Some(Self {
            up: parse_keycode(&input.up)?,
            down: parse_keycode(&input.down)?,
            left: parse_keycode(&input.left)?,
            right: parse_keycode(&input.right)?,
            button_i: parse_keycode(&input.button_i)?,
            button_ii: parse_keycode(&input.button_ii)?,
            rapid_i: parse_keycode(&input.rapid_i)?,
            rapid_ii: parse_keycode(&input.rapid_ii)?,
            select: parse_keycode(&input.select)?,
            run: parse_keycode(&input.run)?,
        })
    }

    pub fn to_set(&self) -> HashSet<Keycode> {
        let mut set = HashSet::new();
        set.insert(self.up);
        set.insert(self.down);
        set.insert(self.left);
        set.insert(self.right);
        set.insert(self.button_i);
        set.insert(self.button_ii);
        set.insert(self.rapid_i);
        set.insert(self.rapid_ii);
        set.insert(self.select);
        set.insert(self.run);
        set
    }
}

fn parse_keycode(value: &str) -> Option<Keycode> {
    let name = value.trim();
    let lower = name.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "up" => Some(Keycode::Up),
        "down" => Some(Keycode::Down),
        "left" => Some(Keycode::Left),
        "right" => Some(Keycode::Right),
        "return" | "enter" => Some(Keycode::Return),
        "space" => Some(Keycode::Space),
        "lshift" | "shift" => Some(Keycode::LShift),
        "rshift" => Some(Keycode::RShift),
        "lctrl" | "ctrl" | "lcontrol" => Some(Keycode::LCtrl),
        "rctrl" | "rcontrol" => Some(Keycode::RCtrl),
        "lalt" | "alt" => Some(Keycode::LAlt),
        "ralt" => Some(Keycode::RAlt),
        _ => None,
    };
    if mapped.is_some() {
        return mapped;
    }
    Keycode::from_name(name)
}

pub fn parse_config_path(args: &[String]) -> (Option<String>, Vec<String>) {
    let mut config_path = None;
    let mut rest = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            if let Some(path) = iter.next() {
                config_path = Some(path.clone());
            }
            continue;
        }
        if let Some(value) = arg.strip_prefix("--config=") {
            config_path = Some(value.to_string());
            continue;
        }
        rest.push(arg.clone());
    }
    (config_path, rest)
}
