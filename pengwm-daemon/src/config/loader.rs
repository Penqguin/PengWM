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

    Ok((settings, keybinds_from_value(path, &value)?))
}

/// The one keybind decode, shared by startup and reload (one scrape — see
/// `keybinds_from_value`). The strict loader used to extract ONLY a
/// `[keybinds]` section while the startup scrape saw the whole file, so a
/// `reload-config` silently replaced every documented top-level bind with
/// defaults. Now both shapes are one behavior wherever the file is read.
pub fn keybinds_from_value(
    path: &Path,
    value: &toml::Value,
) -> Result<KeybindConfig, ConfigError> {
    // The docs' primary convention is top-level `key = "action"` rows.
    let mut config =
        crate::config::keybinds::try_from_toml_value(value).map_err(|e| {
            ConfigError(format!("Invalid keybinds in '{}': {}", path.display(), e))
        })?;
    // The optional `[keybinds]` table adds its own rows on top.
    if let Some(section) = value.get("keybinds") {
        let extra = crate::config::keybinds::try_from_toml_value(section).map_err(|e| {
            ConfigError(format!(
                "Invalid [keybinds] section in '{}': {}",
                path.display(),
                e
            ))
        })?;
        config.bindings.extend(extra.bindings);
    }
    // Zero decoded rows = the file never speaks a keybind — fall back to
    // the built-in set, announced. The decode used to return an empty
    // binding list for a present-but-keybindless file, and every chord
    // (alt-1.., alt-h/j/k/l…) went from "matched" to pass-through with
    // nothing explainable in the log but the missing `Keybind matched`
    // line. A present file is not an opt-out; it just isn't opt-in
    // either.
    if config.bindings.is_empty() {
        log::info!(
            "No keybinds defined in '{}' — using the built-in default set.",
            path.display()
        );
        return Ok(KeybindConfig::default());
    }
    Ok(config)
}

/// The startup-shape decode — the one place "I can't use the config file"
/// becomes "built-in defaults, out loud". Historically the four startup
/// call sites each had their own recovery policy and their own log line
/// (one of them nearly silent), and nothing marked the decoded file as
/// absent — the user running with `gap_outer = 5` in a file the daemon
/// never found learned nothing. Recovery is recoverable, but never
/// default-quiet: every fallback names the path and the docs.
///
/// The reload path keeps its strict sibling (`load`): there a failure
/// keeps the previous config instead — the file the daemon is already
/// running on is the better fall-through, and "everyone's binds vanished"
/// beats "retry loop during an editor save".
pub fn resolve() -> (Settings, KeybindConfig) {
    resolve_from(&config_file_path())
}

pub fn resolve_from(path: &Path) -> (Settings, KeybindConfig) {
    match load_from(path) {
        Ok(pair) => pair,
        Err(e) => {
            log::warn!("Config unusable, running built-in defaults (docs/configuration.md): {e}. Fix it, then `pengwm reload-config`.");
            (Settings::default(), KeybindConfig::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use pengwm_core::command::Command;
    use pengwm_core::tree::Direction;

    fn temp_config(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "pengwm-loader-test-{}-{}",
            std::process::id(),
            name
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    /// The regression this exists for: the reload path used to extract
    /// ONLY a `[keybinds]` section, so a documented top-level
    /// `alt-h = "focus-left"` file — the docs' primary convention —
    /// silently reloaded as default keybinds.
    #[test]
    fn top_level_keybinds_survive_the_decode() {
        let path = temp_config(
            "toplevel.toml",
            r#"
gap_outer = 5
alt-h = "focus-left"
alt-1 = "workspace-1"
"#,
        );
        let (settings, keybinds) = load_from(&path).expect("a valid file decodes fully");
        assert_eq!(settings.gap_outer, 5);
        let bindings = &keybinds.bindings;
        assert!(
            bindings
                .iter()
                .any(|b| b.action == Command::Focus { direction: Direction::Left }),
            "top-level alt-h = focus-left decoded"
        );
        assert_eq!(
            bindings.iter().find(|b| matches!(b.action, Command::Workspace { .. })).map(|b| matches!(
                b.action,
                Command::Workspace { id: 1 }
            )),
            Some(true),
            "top-level alt-1 = workspace-1 decoded"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn keybinds_section_and_top_level_merge() {
        let path = temp_config(
            "merged.toml",
            r#"
gap_outer = 5
alt-h = "focus-left"

[keybinds]
alt-j = "focus-down"
"#,
        );
        let (_, keybinds) = load_from(&path).expect("both shapes decode");
        assert!(
            keybinds
                .bindings
                .iter()
                .any(|b| b.action == Command::Focus { direction: Direction::Left }),
            "the top-level row decoded"
        );
        assert!(
            keybinds
                .bindings
                .iter()
                .any(|b| b.action == Command::Focus { direction: Direction::Down }),
            "the [keybinds] row decoded"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn malformed_toml_is_an_error_not_saved_defaults() {
        let path = temp_config("broken.toml", "gap_outer = = = 5");
        assert!(load_from(&path).is_err(), "reload must see the error");
        // The startup shape defaults — but its recovery is recoverable
        // (`resolve`), never a quiet private re-read.
        let (settings, _) = resolve_from(&path);
        assert_eq!(settings.gap_outer, 10, "defaults carry the built-in gap");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn keybindless_file_still_carries_the_default_set() {
        // The decode regression: a present-but-keybindless file returned
        // an empty binding list, and every default chord went from
        // "matched" to pass-through (the user's alt-1..4 stopped
        // switching workspaces once the gap-only config landed). A file
        // that never speaks a keybind gets the built-in set, like a
        // missing file.
        let path = temp_config("gaps-only.toml", "gap_outer = 5\n");
        let (_, keybinds) = load_from(&path).expect("a valid file decodes");
        assert!(
            keybinds
                .bindings
                .iter()
                .any(|b| matches!(b.action, Command::Workspace { id: 1 })),
            "a present keybindless file must carry the built-in default set"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_is_defaults_and_keybind_defaults() {
        let path = std::env::temp_dir().join(format!(
            "pengwm-loader-missing-{}.toml",
            std::process::id()
        ));
        let (settings, keybinds) = load_from(&path).expect("clean-install state");
        assert_eq!(settings.gap_outer, 10);
        assert!(
            !keybinds.bindings.is_empty(),
            "defaults carry the built-in keybind set"
        );
    }
}
