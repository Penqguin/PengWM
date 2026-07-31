use pengwm_core::command::BarPosition;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const DEFAULT_THEME: &str = "tokyo-night";

/// Per-window color overrides applied on top of the resolved theme. Mirrors
/// ghostty's "set colors directly in the config" escape hatch.
#[derive(Debug, Clone, Default, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct BarConfig {
    #[serde(default = "default_position")]
    pub position: BarPosition,
    #[serde(default = "default_thickness")]
    pub thickness: i32,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default = "default_true")]
    #[allow(dead_code)]
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
            enabled: true,
            theme: DEFAULT_THEME.into(),
            corner_radius: None,
            colors: None,
        }
    }
}

impl BarConfig {
    pub fn load() -> Self {
        Self::load_from(&config_file_path())
    }

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

/// Standard macOS window corner radius (in points) per major version. `10`
/// from Big Sur through Sequoia (11–15), `26` on Tahoe (26, Liquid Glass),
/// `20` on Golden Gate (27). Falls back to `20` for unknown/future majors.
pub fn corner_radius_for_macos(major: u32) -> f32 {
    match major {
        0..=15 => 10.0,
        26 => 26.0,
        _ => 20.0,
    }
}

/// Resolve the effective corner radius: an explicit `[bar].corner_radius`
/// wins, otherwise the macOS-version default is used.
pub fn resolve_corner_radius(config: &BarConfig) -> f32 {
    if let Some(r) = config.corner_radius {
        return r.max(0.0);
    }
    #[cfg(target_os = "macos")]
    {
        corner_radius_for_macos(crate::macos::macos_major_version())
    }
    #[cfg(not(target_os = "macos"))]
    {
        corner_radius_for_macos(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(name: &str, contents: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("pengwm-bar-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn defaults_are_tokyo_night_top_32() {
        let cfg = BarConfig::default();
        assert_eq!(cfg.position, BarPosition::Top);
        assert_eq!(cfg.thickness, 32);
        assert!(cfg.visible);
        assert!(cfg.enabled);
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
    fn corner_radius_table_matches_os_versions() {
        assert_eq!(corner_radius_for_macos(11), 10.0);
        assert_eq!(corner_radius_for_macos(14), 10.0);
        assert_eq!(corner_radius_for_macos(15), 10.0);
        assert_eq!(corner_radius_for_macos(26), 26.0);
        assert_eq!(corner_radius_for_macos(27), 20.0);
        assert_eq!(corner_radius_for_macos(99), 20.0);
    }

    #[test]
    fn explicit_corner_radius_overrides_auto() {
        let cfg = BarConfig {
            corner_radius: Some(6.0),
            ..Default::default()
        };
        assert_eq!(resolve_corner_radius(&cfg), 6.0);
        let cfg = BarConfig {
            corner_radius: Some(-2.0),
            ..Default::default()
        };
        assert_eq!(resolve_corner_radius(&cfg), 0.0);
    }

    #[test]
    fn auto_corner_radius_resolves_from_version() {
        let cfg = BarConfig::default();
        let r = resolve_corner_radius(&cfg);
        assert!(r > 0.0);
    }
}
