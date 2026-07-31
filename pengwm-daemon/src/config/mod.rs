pub mod keybinds;
pub mod watcher;

pub use pengwm_core::command::BarPosition;
use serde::{Deserialize, Serialize};

/// Configuration for the `pengwm-bar` process. The bar reads this same
/// table itself for its theme; the daemon only needs the geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarConfig {
    #[serde(default)]
    pub position: BarPosition,
    #[serde(default = "default_thickness")]
    pub thickness: i32,
    #[serde(default = "default_true")]
    pub visible: bool,
    /// Whether the daemon spawns the `pengwm-bar` process at startup at all.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_thickness() -> i32 {
    32
}

fn default_true() -> bool {
    true
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            position: BarPosition::Top,
            thickness: 32,
            visible: true,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_gap")]
    pub gap_outer: i32,
    #[serde(default = "default_gap_inner")]
    pub gap_inner: i32,
    #[serde(default = "default_max_tiles")]
    pub max_tiles: usize,
    #[serde(default = "default_mod_key")]
    pub mod_key: String,
    #[serde(default)]
    pub restricted_apps: Vec<String>,
    #[serde(default)]
    pub bar: BarConfig,
}

fn default_gap() -> i32 {
    10
}

fn default_gap_inner() -> i32 {
    5
}

fn default_max_tiles() -> usize {
    4
}

fn default_mod_key() -> String {
    "cmd".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gap_outer: 10,
            gap_inner: 5,
            max_tiles: 4,
            mod_key: "cmd".into(),
            restricted_apps: Vec::new(),
            bar: BarConfig::default(),
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
            Ok(contents) => match toml::from_str::<Settings>(&contents) {
                Ok(settings) => settings,
                Err(e) => {
                    log::warn!(
                        "Failed to parse config '{}': {}. Using defaults.",
                        path.display(),
                        e
                    );
                    Self::default()
                }
            },
            Err(_) => {
                log::info!("No config file at '{}'. Using defaults.", path.display());
                Self::default()
            }
        }
    }
}

pub fn config_file_path() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        let path = std::path::PathBuf::from(dir)
            .join("pengwm")
            .join("config.toml");
        if path.exists() {
            return path;
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home)
        .join(".config")
        .join("pengwm")
        .join("config.toml")
}
