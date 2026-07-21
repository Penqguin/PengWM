//! Configuration loading and representation.
//!
//! Reads a TOML file at $XDG_CONFIG_HOME/pengwm/config.toml
//! (fallback: ~/.config/pengwm/config.toml).

use serde::{Serialize, Deserialize};

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Full config struct, populated from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    // pub gap_outer: i32,
    // pub gap_inner: i32,
    // pub max_tiles: usize,
    // pub keybinds: KeybindConfig,
    // pub layouts: LayoutConfig,
    // pub mod_key: String,   // "cmd", "alt", "ctrl", etc.
}

impl Default for Settings {
    fn default() -> Self {
        //  gap_outer: 10
        //  gap_inner: 5
        //  max_tiles: 4
        //  mod_key: "cmd"
        //  keybinds: KeybindConfig::default()
        todo!("return sensible defaults")
    }
}

impl Settings {
    /// Load from the default config file path.
    pub fn load() -> Self {
        //  determine config dir (XDG_CONFIG_HOME or ~/.config)
        //  read file as string
        //  toml::from_str(&contents)
        //  on error, log warning and return Default
        todo!()
    }

    /// Load from an explicit path.
    pub fn load_from(path: &std::path::Path) -> Self {
        todo!()
    }
}

// ---------------------------------------------------------------------------
// Config path
// ---------------------------------------------------------------------------

/// Return the path to the config file.
pub fn config_file_path() -> std::path::PathBuf {
    todo!("$XDG_CONFIG_HOME/pengwm/config.toml or ~/.config/pengwm/config.toml")
}
