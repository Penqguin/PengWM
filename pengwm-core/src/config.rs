use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The shared `[bar]` configuration table, parsed identically by the daemon
/// (for geometry + spawning) and by `pengwm-bar` (for rendering). One
/// definition, one set of defaults — a single source of truth for where the
/// bar sits and how it looks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarConfig {
    #[serde(default = "default_position")]
    pub position: BarPosition,
    #[serde(default = "default_thickness")]
    pub thickness: i32,
    #[serde(default = "default_true")]
    pub visible: bool,
    /// Whether the daemon spawns the `pengwm-bar` process at startup at all.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Explicit corner radius in points. `None` = auto-detect from the macOS
    /// version.
    #[serde(default)]
    pub corner_radius: Option<f32>,
    #[serde(default)]
    pub colors: Option<ColorOverrides>,
}

fn default_position() -> BarPosition {
    BarPosition::Top
}

fn default_thickness() -> i32 {
    32
}

fn default_true() -> bool {
    true
}

fn default_theme() -> String {
    DEFAULT_THEME.into()
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            position: BarPosition::Top,
            thickness: 32,
            visible: true,
            enabled: false,
            theme: DEFAULT_THEME.into(),
            corner_radius: None,
            colors: None,
        }
    }
}

impl BarConfig {
    /// Read the `[bar]` table from the default config path.
    pub fn load() -> Self {
        Self::load_from(&config_file_path())
    }

    /// Read the `[bar]` table from `path`. Any failure (missing file, invalid
    /// TOML, malformed table) falls back to defaults, mirroring how the daemon
    /// treats the whole config file.
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => match contents.parse::<toml::Value>() {
                Ok(value) => match value.get("bar").map(|v| v.clone().try_into::<BarConfig>()) {
                    Some(Ok(cfg)) => cfg,
                    Some(Err(e)) => {
                        log::warn!(
                            "Failed to parse [bar] config in '{}': {}. Using defaults.",
                            path.display(),
                            e
                        );
                        Self::default()
                    }
                    None => Self::default(),
                },
                Err(e) => {
                    log::warn!(
                        "Failed to parse '{}': {}. Using defaults.",
                        path.display(),
                        e
                    );
                    Self::default()
                }
            },
            Err(_) => {
                log::debug!("No config at '{}'. Using defaults.", path.display());
                Self::default()
            }
        }
    }
}

/// Which edge of the display the bar strip occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

/// Per-window color overrides applied on top of the resolved theme. Mirrors
/// ghostty's "set colors directly in the config" escape hatch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColorOverrides {
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub foreground: Option<String>,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub inactive: Option<String>,
    #[serde(default)]
    pub border: Option<String>,
    #[serde(default)]
    pub font_size: Option<f32>,
}

pub const DEFAULT_THEME: &str = "tokyo-night";

/// The pengwm config file path (`$XDG_CONFIG_HOME/pengwm/config.toml`, falling
/// back to `~/.config/pengwm/config.toml`). Shared so the daemon and bar read
/// the same file.
pub fn config_file_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(dir).join("pengwm").join("config.toml");
        if path.exists() {
            return path;
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".config")
        .join("pengwm")
        .join("config.toml")
}

/// Directory for user-provided theme files (`~/.config/pengwm/themes/`).
pub fn themes_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(dir).join("pengwm").join("themes");
        if path.exists() {
            return path;
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".config")
        .join("pengwm")
        .join("themes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("pengwm-core-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn defaults_are_top_32() {
        let cfg = BarConfig::default();
        assert_eq!(cfg.position, BarPosition::Top);
        assert_eq!(cfg.thickness, 32);
        assert!(cfg.visible);
        assert!(!cfg.enabled);
        assert_eq!(cfg.theme, "tokyo-night");
        assert_eq!(cfg.corner_radius, None);
    }

    #[test]
    fn parses_full_bar_table() {
        let path = write_tmp(
            "full.toml",
            r#"
gap_outer = 8
[bar]
position = "bottom"
thickness = 40
visible = false
theme = "nord"
corner_radius = 12
"#,
        );
        let cfg = BarConfig::load_from(&path);
        assert_eq!(cfg.position, BarPosition::Bottom);
        assert_eq!(cfg.thickness, 40);
        assert!(!cfg.visible);
        assert_eq!(cfg.theme, "nord");
        assert_eq!(cfg.corner_radius, Some(12.0));
    }

    #[test]
    fn missing_bar_table_uses_defaults() {
        let path = write_tmp("nobar.toml", "gap_outer = 8\n");
        let cfg = BarConfig::load_from(&path);
        assert_eq!(cfg.position, BarPosition::Top);
        assert_eq!(cfg.theme, DEFAULT_THEME);
    }

    #[test]
    fn missing_file_uses_defaults() {
        let cfg = BarConfig::load_from(&PathBuf::from("/nonexistent/pengwm.toml"));
        assert_eq!(cfg.theme, DEFAULT_THEME);
    }

    #[test]
    fn invalid_file_uses_defaults() {
        let path = write_tmp("bad.toml", "not [ valid toml \n");
        let cfg = BarConfig::load_from(&path);
        assert_eq!(cfg.theme, DEFAULT_THEME);
    }

    #[test]
    fn bar_position_roundtrips_through_toml() {
        let cfg = BarConfig {
            position: BarPosition::Left,
            ..Default::default()
        };
        let value = toml::to_string(&cfg).unwrap();
        let back: BarConfig = toml::from_str(&value).unwrap();
        assert_eq!(back.position, BarPosition::Left);
    }
}
