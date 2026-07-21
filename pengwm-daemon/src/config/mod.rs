pub mod keybinds;

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub gap_outer: i32,
    pub gap_inner: i32,
    pub max_tiles: usize,
    pub mod_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gap_outer: 10,
            gap_inner: 5,
            max_tiles: 4,
            mod_key: "cmd".into(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let path = config_file_path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                match toml::from_str::<Settings>(&contents) {
                    Ok(settings) => settings,
                    Err(e) => {
                        log::warn!("Failed to parse config '{}': {}. Using defaults.", path.display(), e);
                        Self::default()
                    }
                }
            }
            Err(_) => {
                log::info!("No config file at '{}'. Using defaults.", path.display());
                Self::default()
            }
        }
    }
}

pub fn config_file_path() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        let path = std::path::PathBuf::from(dir).join("pengwm").join("config.toml");
        if path.exists() {
            return path;
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".config").join("pengwm").join("config.toml")
}
