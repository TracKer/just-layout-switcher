use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

const FILE_NAME: &str = "config.yaml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub switch_mode: SwitchMode,
    /// Human-readable key combination, e.g. "Ctrl + Shift".
    pub hotkey: String,
    /// Swallow the hotkey after switching instead of passing it on to the
    /// focused application and other hooks.
    pub block_hotkey: bool,
    /// User-defined switching order; empty means the system order.
    pub layouts: Vec<LayoutOrder>,
}

/// How the hotkey picks the layout to switch to. Disabled layouts are
/// skipped in every mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchMode {
    /// Each press switches to the next layout, wrapping from the last to the first.
    #[default]
    Cycle,
    /// N presses in a row switch to the N-th enabled layout.
    /// TODO: not implemented yet.
    PressCount,
    /// Every layout has its own hotkey, set in its options dialog ("..." in
    /// the Layouts list); the common hotkey is unused.
    /// TODO: not implemented yet.
    PerLayout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutOrder {
    /// Layout handle (HKL) as 8 hex digits, e.g. "04090409".
    pub id: String,
    /// Position in the list, starting from 1 at the top.
    pub order: u32,
    /// Disabled layouts keep their position but are skipped when switching.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            switch_mode: SwitchMode::Cycle,
            hotkey: "Alt + Shift".to_string(),
            block_hotkey: true,
            layouts: Vec::new(),
        }
    }
}

/// The config lives next to the executable.
pub fn path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(FILE_NAME)))
        .unwrap_or_else(|| PathBuf::from(FILE_NAME))
}

/// Loads the config, creating a default file if there is none yet.
/// A malformed file is left untouched and defaults are used.
pub fn load() -> Config {
    let path = path();
    match fs::read_to_string(&path) {
        Ok(text) => serde_yaml::from_str(&text).unwrap_or_default(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let config = Config::default();
            let _ = save(&config);
            config
        }
        Err(_) => Config::default(),
    }
}

pub fn save(config: &Config) -> io::Result<()> {
    let text = serde_yaml::to_string(config).map_err(io::Error::other)?;
    fs::write(path(), text)
}
