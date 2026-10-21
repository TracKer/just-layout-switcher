use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

const FILE_NAME: &str = "config.yaml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Human-readable key combination, e.g. "Ctrl + Shift".
    pub hotkey: String,
    /// User-defined switching order; empty means the system order.
    pub layouts: Vec<LayoutOrder>,
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
            hotkey: "Alt + Shift".to_string(),
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
