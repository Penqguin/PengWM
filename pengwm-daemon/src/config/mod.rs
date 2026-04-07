//! Configuration management for PengWM.
//! Handles loading from config.toml and watching for changes.

use serde::{Serialize, Deserialize};
use std::fs;
use std::path::Path;

/// File system watcher for automatic configuration reloading.
pub mod watcher;

/// Global configuration for the window manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Maximum number of tiles per workspace.
    pub max_tiles: usize,
    /// Margin between windows and the monitor edge.
    pub gap_outer: i32,
    /// Margin between adjacent windows.
    pub gap_inner: i32,
}

impl Config {
    /// Loads configuration from the standard `config.toml` file.
    /// If the file doesn't exist, it returns the default configuration.
    pub fn load() -> Self {
        let path = Path::new("config.toml");
        if path.exists() {
            match fs::read_to_string(path) {
                Ok(content) => {
                    match toml::from_str(&content) {
                        Ok(config) => {
                            log::info!("Loaded configuration from config.toml");
                            return config;
                        },
                        Err(e) => log::error!("Failed to parse config.toml: {}", e),
                    }
                }
                Err(e) => log::error!("Failed to read config.toml: {}", e),
            }
        }
        
        log::info!("Using default configuration");
        Self::default()
    }
}

impl Default for Config {
    /// Returns the default configuration for PengWM.
    fn default() -> Self {
        Self {
            max_tiles: 4,
            gap_outer: 10,
            gap_inner: 5,
        }
    }
}
