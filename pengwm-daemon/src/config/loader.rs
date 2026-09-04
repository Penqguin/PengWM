use std::path::Path;

use crate::config::keybinds::KeybindConfig;
use crate::config::{config_file_path, Settings};

#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for ConfigError {}

/// Single-read loader: reads the config file once and splits into Settings +
/// KeybindConfig. Missing file -> Ok(defaults) (clean install). Malformed
/// TOML or invalid field -> Err (caller logs and keeps old config).
pub fn load() -> Result<(Settings, KeybindConfig), ConfigError> {
    load_from(&config_file_path())
}

pub fn load_from(path: &Path) -> Result<(Settings, KeybindConfig), ConfigError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!("No config file at '{}'. Using defaults.", path.display());
            return Ok((Settings::default(), KeybindConfig::default()));
        }
        Err(e) => {
            return Err(ConfigError(format!(
                "Failed to read config '{}': {}",
                path.display(),
                e
            )))
        }
    };

    let value: toml::Value = contents.parse().map_err(|e: toml::de::Error| {
        ConfigError(format!(
            "Failed to parse config '{}': {}",
            path.display(),
            e
        ))
    })?;

    let settings: Settings = value
        .clone()
        .try_into()
        .map_err(|e| ConfigError(format!("Invalid settings in '{}': {}", path.display(), e)))?;

    let keybinds = match value.get("keybinds") {
        Some(v) => crate::config::keybinds::try_from_toml_value(v).map_err(|e| {
            ConfigError(format!("Invalid [keybinds] in '{}': {}", path.display(), e))
        })?,
        None => KeybindConfig::default(),
    };

    Ok((settings, keybinds))
}
