use crate::config::{themes_dir, BarConfig, ColorOverrides};
use eframe::egui::Color32;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Color32,
    pub foreground: Color32,
    pub accent: Color32,
    pub inactive: Color32,
    pub border: Color32,
    pub font_size: f32,
}

impl Default for Theme {
    fn default() -> Self {
        tokyo_night()
    }
}

pub fn tokyo_night() -> Theme {
    Theme {
        background: hex("#1a1b26"),
        foreground: hex("#c0caf5"),
        accent: hex("#7aa2f7"),
        inactive: hex("#3b4261"),
        border: hex("#565f89"),
        font_size: 12.0,
    }
}

/// Resolve the effective theme: user theme file (path or `themes/` dir),
/// falling back to a built-in, then `[bar].colors` overrides.
pub fn resolve(config: &BarConfig) -> Theme {
    let mut theme = match resolve_source(&config.theme) {
        ThemeSource::File(path) => match Theme::from_toml_file(&path) {
            Some(t) => t,
            None => {
                log::warn!("Theme file '{}' unreadable; using built-in", path.display());
                builtin(&config.theme).unwrap_or_default()
            }
        },
        ThemeSource::Builtin => match builtin(&config.theme) {
            Some(t) => t,
            None => {
                log::warn!(
                    "Unknown bar theme '{}'; using default '{}'",
                    config.theme,
                    crate::config::DEFAULT_THEME
                );
                Theme::default()
            }
        },
    };

    if let Some(colors) = &config.colors {
        apply_overrides(&mut theme, colors);
    }
    theme
}

enum ThemeSource {
    File(PathBuf),
    Builtin,
}

fn resolve_source(theme: &str) -> ThemeSource {
    if looks_like_path(theme) {
        return ThemeSource::File(expand_tilde(theme));
    }
    let candidate = themes_dir().join(format!("{theme}.toml"));
    if candidate.exists() {
        ThemeSource::File(candidate)
    } else {
        ThemeSource::Builtin
    }
}

fn looks_like_path(s: &str) -> bool {
    s.starts_with('/') || s.starts_with('.') || s.starts_with('~') || s.contains('/')
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(s)
}

fn builtin(name: &str) -> Option<Theme> {
    let t = match name.to_ascii_lowercase().as_str() {
        "tokyo-night" => tokyo_night(),
        "catppuccin-mocha" => Theme {
            background: hex("#1e1e2e"),
            foreground: hex("#cdd6f4"),
            accent: hex("#89b4fa"),
            inactive: hex("#45475a"),
            border: hex("#6c7086"),
            font_size: 12.0,
        },
        "catppuccin-latte" => Theme {
            background: hex("#eff1f5"),
            foreground: hex("#4c4f69"),
            accent: hex("#1e66f5"),
            inactive: hex("#ccd0da"),
            border: hex("#9ca0b0"),
            font_size: 12.0,
        },
        "nord" => Theme {
            background: hex("#2e3440"),
            foreground: hex("#d8dee9"),
            accent: hex("#88c0d0"),
            inactive: hex("#434c5e"),
            border: hex("#4c566a"),
            font_size: 12.0,
        },
        "dracula" => Theme {
            background: hex("#282a36"),
            foreground: hex("#f8f8f2"),
            accent: hex("#bd93f9"),
            inactive: hex("#44475a"),
            border: hex("#6272a4"),
            font_size: 12.0,
        },
        "one-dark" => Theme {
            background: hex("#282c34"),
            foreground: hex("#abb2bf"),
            accent: hex("#61afef"),
            inactive: hex("#3e4451"),
            border: hex("#4b5263"),
            font_size: 12.0,
        },
        "solarized-dark" => Theme {
            background: hex("#002b36"),
            foreground: hex("#839496"),
            accent: hex("#268bd2"),
            inactive: hex("#073642"),
            border: hex("#586e75"),
            font_size: 12.0,
        },
        "solarized-light" => Theme {
            background: hex("#fdf6e3"),
            foreground: hex("#657b83"),
            accent: hex("#268bd2"),
            inactive: hex("#eee8d5"),
            border: hex("#93a1a1"),
            font_size: 12.0,
        },
        "gruvbox-dark" => Theme {
            background: hex("#282828"),
            foreground: hex("#ebdbb2"),
            accent: hex("#83a598"),
            inactive: hex("#504945"),
            border: hex("#7c6f64"),
            font_size: 12.0,
        },
        "gruvbox-light" => Theme {
            background: hex("#fbf1c7"),
            foreground: hex("#3c3836"),
            accent: hex("#458588"),
            inactive: hex("#d5c4a1"),
            border: hex("#bdae93"),
            font_size: 12.0,
        },
        "rose-pine" => Theme {
            background: hex("#191724"),
            foreground: hex("#e0def4"),
            accent: hex("#ebbcba"),
            inactive: hex("#403d52"),
            border: hex("#6e6a86"),
            font_size: 12.0,
        },
        "kanagawa" => Theme {
            background: hex("#1f1f28"),
            foreground: hex("#dcd7ba"),
            accent: hex("#7e9cd8"),
            inactive: hex("#363646"),
            border: hex("#54546d"),
            font_size: 12.0,
        },
        _ => return None,
    };
    Some(t)
}

impl Theme {
    pub fn from_toml_file(path: &Path) -> Option<Self> {
        let contents = std::fs::read_to_string(path).ok()?;
        let value: toml::Value = contents.parse().ok()?;
        let table = value.as_table()?;

        let mut theme = Theme::default();
        if let Some(v) = table.get("background").and_then(|v| v.as_str()) {
            if let Some(c) = parse_color(v) {
                theme.background = c;
            }
        }
        if let Some(v) = table.get("foreground").and_then(|v| v.as_str()) {
            if let Some(c) = parse_color(v) {
                theme.foreground = c;
            }
        }
        if let Some(v) = table.get("accent").and_then(|v| v.as_str()) {
            if let Some(c) = parse_color(v) {
                theme.accent = c;
            }
        }
        if let Some(v) = table.get("inactive").and_then(|v| v.as_str()) {
            if let Some(c) = parse_color(v) {
                theme.inactive = c;
            }
        }
        if let Some(v) = table.get("border").and_then(|v| v.as_str()) {
            if let Some(c) = parse_color(v) {
                theme.border = c;
            }
        }
        if let Some(v) = table
            .get("font_size")
            .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        {
            theme.font_size = v as f32;
        }
        Some(theme)
    }
}

fn apply_overrides(theme: &mut Theme, overrides: &ColorOverrides) {
    if let Some(c) = &overrides.background {
        theme.background = parse_color(c).unwrap_or(theme.background);
    }
    if let Some(c) = &overrides.foreground {
        theme.foreground = parse_color(c).unwrap_or(theme.foreground);
    }
    if let Some(c) = &overrides.accent {
        theme.accent = parse_color(c).unwrap_or(theme.accent);
    }
    if let Some(c) = &overrides.inactive {
        theme.inactive = parse_color(c).unwrap_or(theme.inactive);
    }
    if let Some(c) = &overrides.border {
        theme.border = parse_color(c).unwrap_or(theme.border);
    }
    if let Some(fs) = overrides.font_size {
        theme.font_size = fs;
    }
}

pub fn hex(s: &str) -> Color32 {
    parse_color(s).unwrap_or(Color32::WHITE)
}

/// Parse `#RRGGBB` or `#RRGGBBAA` (the `#` is optional).
pub fn parse_color(s: &str) -> Option<Color32> {
    let s = s.trim().trim_start_matches('#');
    let (rgb, alpha) = match s.len() {
        6 => (&s[..6], 255),
        8 => (&s[..6], u8::from_str_radix(&s[6..], 16).ok()?),
        _ => return None,
    };
    if let Ok(v) = u32::from_str_radix(rgb, 16) {
        Some(Color32::from_rgba_premultiplied(
            ((v >> 16) & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            (v & 0xff) as u8,
            alpha,
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_rgb() {
        let c = parse_color("#1a1b26").unwrap();
        assert_eq!(c.r(), 0x1a);
        assert_eq!(c.g(), 0x1b);
        assert_eq!(c.b(), 0x26);
        assert_eq!(c.a(), 255);
    }

    #[test]
    fn parse_hex_rgba() {
        let c = parse_color("#1a1b26ff").unwrap();
        assert_eq!(c.a(), 255);
    }

    #[test]
    fn parse_hex_without_hash() {
        assert_eq!(parse_color("1a1b26"), parse_color("#1a1b26"));
    }

    #[test]
    fn parse_hex_invalid() {
        assert!(parse_color("zzz").is_none());
        assert!(parse_color("#12345").is_none());
    }

    #[test]
    fn default_is_tokyo_night() {
        let t = Theme::default();
        assert_eq!(t.background, hex("#1a1b26"));
    }

    #[test]
    fn builtin_names_resolve() {
        for name in [
            "tokyo-night",
            "catppuccin-mocha",
            "nord",
            "dracula",
            "gruvbox-dark",
        ] {
            assert!(builtin(name).is_some(), "{name} should be built in");
        }
        assert!(builtin("nope").is_none());
    }

    #[test]
    fn unknown_theme_falls_back_to_default() {
        let cfg = BarConfig {
            theme: "no-such-theme".into(),
            ..Default::default()
        };
        let theme = resolve(&cfg);
        assert_eq!(theme.background, tokyo_night().background);
    }

    #[test]
    fn colors_override_theme() {
        let cfg = BarConfig {
            colors: Some(ColorOverrides {
                accent: Some("#ff0000".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let theme = resolve(&cfg);
        assert_eq!(theme.accent, hex("#ff0000"));
        assert_eq!(theme.background, tokyo_night().background);
    }

    #[test]
    fn theme_file_loads() {
        let dir = std::env::temp_dir().join("pengwm-bar-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("custom-theme.toml");
        std::fs::write(
            &path,
            "background = \"#000000\"\nforeground = \"#ffffff\"\nfont_size = 14\n",
        )
        .unwrap();
        let theme = Theme::from_toml_file(&path).unwrap();
        assert_eq!(theme.background, hex("#000000"));
        assert_eq!(theme.foreground, hex("#ffffff"));
        assert_eq!(theme.font_size, 14.0);
        assert_eq!(
            theme.accent,
            tokyo_night().accent,
            "missing keys keep defaults"
        );
    }
}
