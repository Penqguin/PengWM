use std::path::PathBuf;

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
