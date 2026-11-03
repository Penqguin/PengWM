use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Which edge of the display a reserved strip (e.g. a status-bar pane)
/// occupies. Pure layout vocabulary — right now nothing sets a reservation,
/// but the workspace geometry supports it for future UI surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

/// The pengwm config file path (`$XDG_CONFIG_HOME/pengwm/config.toml`, falling
/// back to `~/.config/pengwm/config.toml`).
pub fn config_file_path() -> PathBuf {
    // XDG resolution is decisive: when `$XDG_CONFIG_HOME` is set, it owns
    // the file's location whether or not the file exists. Falling back
    // while it is set would let two copies shadow each other — whichever
    // exists silently wins, and a stale copy at the home path can eat a
    // fresh one at the XDG path (or vice versa).
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("pengwm").join("config.toml");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".config")
        .join("pengwm")
        .join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_position_roundtrips_through_toml() {
        // The `toml` crate cannot serialize a bare enum, so roundtrip
        // through a wrapper table — the shape a real config file uses.
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Wrapper {
            position: BarPosition,
        }
        let value = toml::to_string(&Wrapper {
            position: BarPosition::Left,
        })
        .unwrap();
        let back: Wrapper = toml::from_str(&value).unwrap();
        assert_eq!(back.position, BarPosition::Left);
    }
}
