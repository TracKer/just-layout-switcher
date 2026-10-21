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
/// skipped when cycling or counting; their own hotkeys still work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchMode {
    /// Each press switches to the next layout, wrapping from the last to the first.
    #[default]
    Cycle,
    /// N presses in a row switch to the N-th enabled layout; every press
    /// switches right away, a pause starts counting over.
    PressCount,
    /// Every layout has its own hotkey that switches straight to it, set in
    /// its options dialog ("..." in the Layouts list); the common hotkey is unused.
    PerLayout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutOrder {
    /// Layout handle (HKL) as 8 hex digits, e.g. "04090409".
    pub id: String,
    /// Position in the list, starting from 1 at the top.
    pub order: u32,
    /// Disabled layouts keep their position but are skipped when cycling or
    /// counting presses.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// Switches straight to this layout in the per-layout mode; empty if none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hotkey: String,
    /// A turned off hotkey is kept but not tracked.
    #[serde(default = "enabled_by_default", skip_serializing_if = "is_true")]
    pub hotkey_enabled: bool,
}

impl LayoutOrder {
    pub fn new(id: String, order: u32, enabled: bool) -> Self {
        Self { id, order, enabled, hotkey: String::new(), hotkey_enabled: true }
    }
}

fn enabled_by_default() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// What a hotkey is assigned to.
#[derive(Debug, Clone, PartialEq)]
pub enum HotkeyOwner {
    /// The common hotkey on the General page.
    Main,
    /// The hotkey of the layout with this id.
    Layout(String),
}

impl Config {
    /// What else already uses the same key combination as `hotkey`, apart
    /// from `owner` itself. Turned off and currently unused hotkeys count
    /// too, so that turning them on later can't give one combination two
    /// actions; cleared (empty) ones don't.
    pub fn hotkey_user(&self, hotkey: &str, owner: &HotkeyOwner) -> Option<HotkeyOwner> {
        let keys = crate::keys::parse(hotkey)?;
        let same = |text: &str| crate::keys::parse(text).as_ref() == Some(&keys);
        if *owner != HotkeyOwner::Main && same(&self.hotkey) {
            return Some(HotkeyOwner::Main);
        }
        self.layouts
            .iter()
            .find(|l| *owner != HotkeyOwner::Layout(l.id.clone()) && same(&l.hotkey))
            .map(|l| HotkeyOwner::Layout(l.id.clone()))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        let mut config = Config { hotkey: "Alt + Shift".to_string(), ..Config::default() };
        let mut en = LayoutOrder::new("EN".to_string(), 1, true);
        en.hotkey = "Ctrl + 1".to_string();
        let mut ru = LayoutOrder::new("RU".to_string(), 2, true);
        ru.hotkey = "Ctrl + 2".to_string();
        ru.hotkey_enabled = false;
        config.layouts = vec![en, ru, LayoutOrder::new("DE".to_string(), 3, true)];
        config
    }

    #[test]
    fn finds_who_uses_a_hotkey() {
        let config = config();
        let de = HotkeyOwner::Layout("DE".to_string());
        assert_eq!(config.hotkey_user("Shift + Alt", &de), Some(HotkeyOwner::Main));
        assert_eq!(config.hotkey_user("Ctrl + 1", &HotkeyOwner::Main), Some(HotkeyOwner::Layout("EN".to_string())));
        // A turned off hotkey still counts.
        assert_eq!(config.hotkey_user("Ctrl + 2", &de), Some(HotkeyOwner::Layout("RU".to_string())));
        assert_eq!(config.hotkey_user("Ctrl + 3", &de), None);
    }

    #[test]
    fn cleared_hotkeys_clash_with_nothing() {
        let mut config = config();
        assert_eq!(config.hotkey_user("", &HotkeyOwner::Main), None);
        config.layouts[0].hotkey.clear();
        assert_eq!(config.hotkey_user("Ctrl + 1", &HotkeyOwner::Main), None);
    }

    #[test]
    fn own_hotkey_is_not_a_clash() {
        let config = config();
        assert_eq!(config.hotkey_user("Alt + Shift", &HotkeyOwner::Main), None);
        assert_eq!(config.hotkey_user("Ctrl + 1", &HotkeyOwner::Layout("EN".to_string())), None);
    }

    #[test]
    fn layout_hotkey_fields_are_optional_in_the_file() {
        let config: Config = serde_yaml::from_str("layouts:\n- id: EN\n  order: 1\n").unwrap();
        assert_eq!(config.layouts[0].hotkey, "");
        assert!(config.layouts[0].hotkey_enabled);
        let text = serde_yaml::to_string(&config).unwrap();
        assert!(!text.contains("hotkey_enabled"));
    }
}
