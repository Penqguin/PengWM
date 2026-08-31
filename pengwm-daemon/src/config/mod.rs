pub mod keybinds;
pub mod watcher;

pub use pengwm_core::config::{config_file_path, BarConfig, BarPosition};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_gap")]
    pub gap_outer: i32,
    #[serde(default = "default_gap_inner")]
    pub gap_inner: i32,
    #[serde(default = "default_max_tiles")]
    pub max_tiles: usize,
    #[serde(default)]
    pub restricted_apps: Vec<String>,
    #[serde(default)]
    pub bar: BarConfig,
    #[serde(default)]
    pub menubar: MenubarConfig,
    /// The named workspaces created on every monitor at startup, in order.
    /// Windows launched by an app listed in an entry's `apps` are routed into
    /// that workspace. Defaults to five named workspaces.
    #[serde(default = "default_workspaces")]
    pub workspaces: Vec<WorkspaceEntry>,
}

/// One named workspace and the apps (by bundle id or app name) whose windows
/// should be routed into it. `apps` is matched case-insensitively against each
/// app's bundle id, falling back to its display name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub name: String,
    #[serde(default)]
    pub apps: Vec<String>,
}

pub fn default_workspaces() -> Vec<WorkspaceEntry> {
    vec![
        WorkspaceEntry {
            name: "Development".into(),
            apps: vec![
                "com.apple.dt.Xcode".into(),
                "com.googlecode.iterm2".into(),
                "com.microsoft.VSCode".into(),
                "com.apple.Terminal".into(),
                "ghostty".into(),
                "com.warp.Warp".into(),
                "Xcode".into(),
                "iTerm2".into(),
                "Code".into(),
                "Terminal".into(),
                "Ghostty".into(),
                "Warp".into(),
                "kitty".into(),
                "Alacritty".into(),
                "zed".into(),
            ],
        },
        WorkspaceEntry {
            name: "Browsing".into(),
            apps: vec![
                "com.apple.Safari".into(),
                "com.google.Chrome".into(),
                "org.mozilla.firefox".into(),
                "company.thebrowser.Browser".into(),
                "com.microsoft.edgemac".into(),
                "com.brave.Browser".into(),
                "Safari".into(),
                "Chrome".into(),
                "Firefox".into(),
                "Arc".into(),
                "Edge".into(),
                "Brave".into(),
                "Opera".into(),
            ],
        },
        WorkspaceEntry {
            name: "Notes".into(),
            apps: vec![
                "com.apple.Notes".into(),
                "md.obsidian".into(),
                "com.github.notion".into(),
                "Notes".into(),
                "Obsidian".into(),
                "Notion".into(),
            ],
        },
        WorkspaceEntry {
            name: "Music".into(),
            apps: vec![
                "com.apple.Music".into(),
                "com.spotify.client".into(),
                "Music".into(),
                "Spotify".into(),
            ],
        },
        WorkspaceEntry {
            name: "Messaging".into(),
            apps: vec![
                "com.apple.MobileSMS".into(),
                "com.apple.iChat".into(),
                "com.tinyspeck.slackmacgap".into(),
                "com.hnc.Discord".into(),
                "com.whatsapp.WhatsApp".into(),
                "com.tencent.xinWeChat".into(),
                "Messages".into(),
                "Slack".into(),
                "Discord".into(),
                "WhatsApp".into(),
                "WeChat".into(),
                "Telegram".into(),
            ],
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenubarConfig {
    /// Whether the daemon spawns `pengwm-menubar` (the status-bar icon whose
    /// menu lists workspaces and the app names inside them).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for MenubarConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
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

impl Default for Settings {
    fn default() -> Self {
        Self {
            gap_outer: 10,
            gap_inner: 5,
            max_tiles: 4,
            restricted_apps: Vec::new(),
            bar: BarConfig::default(),
            menubar: MenubarConfig::default(),
            workspaces: default_workspaces(),
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
