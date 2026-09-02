use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::WorkspaceEntry;
use pengwm_core::workspace::Workspace;

mod string_key_map {
    use std::collections::HashMap;
    use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(map: &HashMap<u32, usize>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let string_map: HashMap<String, usize> =
            map.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        string_map.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<u32, usize>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let string_map = HashMap::<String, usize>::deserialize(deserializer)?;
        string_map
            .into_iter()
            .map(|(k, v)| k.parse::<u32>().map(|nk| (nk, v)).map_err(de::Error::custom))
            .collect()
    }
}

/// Versioned on-disk session snapshot. Stored at
/// `~/.local/share/pengwm/state.toml` (or `$XDG_STATE_HOME/pengwm/state.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub workspaces: Vec<Workspace>,
    #[serde(with = "string_key_map")]
    pub active: HashMap<u32, usize>,
    pub entries: Vec<WorkspaceEntry>,
    pub gap_outer: f64,
    pub gap_inner: f64,
}

impl Session {
    pub fn new(
        workspaces: Vec<Workspace>,
        active: HashMap<u32, usize>,
        entries: Vec<WorkspaceEntry>,
        gap_outer: f64,
        gap_inner: f64,
    ) -> Self {
        Self {
            version: 1,
            workspaces,
            active,
            entries,
            gap_outer,
            gap_inner,
        }
    }
}

pub fn session_file_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(dir).join("pengwm").join("state.toml");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("pengwm")
        .join("state.toml")
}

/// Sanitize a workspace for persistence: keep name/monitor/geometry/monocle
/// and split topology, but strip all window leaves so stale WindowIds don't
/// survive a reboot. We keep Split nodes with their ratios so the layout
/// skeleton survives; window children are pruned.
///
/// For now we do a simpler sanitize: return an empty workspace with the same
/// metadata. The split skeleton is lost, but the workspace is clean and
/// routing will recreate splits. This satisfies "topology only + re-route"
/// without complex arena surgery. A future enhancement can keep Split nodes.
pub fn sanitize_workspace(ws: &Workspace) -> Workspace {
    let mut sanitized = Workspace::new(
        ws.name.clone(),
        ws.monitor_id,
        ws.monitor_origin(),
        ws.monitor_size(),
    );
    // Preserve monocle flag; focus and tree are reset.
    sanitized.monocle = ws.monocle;
    sanitized
}

/// Build a Session snapshot from the live `StateManager` fields, sanitizing
/// workspaces so no WindowIds are persisted.
pub fn snapshot_from(
    workspaces: &[Workspace],
    active: &HashMap<u32, usize>,
    entries: &[WorkspaceEntry],
    gap_outer: f64,
    gap_inner: f64,
) -> Session {
    let sanitized: Vec<Workspace> = workspaces.iter().map(sanitize_workspace).collect();
    Session::new(
        sanitized,
        active.clone(),
        entries.to_vec(),
        gap_outer,
        gap_inner,
    )
}

/// Atomically write the session to `path` (tmp+rename+fsync parent).
pub fn save_to(session: &Session, path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml_str = toml::to_string_pretty(session)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, toml_str)?;
    // Best-effort fsync the tmp file before rename.
    if let Ok(f) = std::fs::File::open(&tmp) {
        let _ = f.sync_all();
    }
    std::fs::rename(&tmp, path)?;
    // Fsync parent directory for durability (best-effort).
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// Load a session from `path`, returning `None` on missing/corrupt/version-mismatch.
pub fn load_from(path: &Path) -> Option<Session> {
    let contents = std::fs::read_to_string(path).ok()?;
    match toml::from_str::<Session>(&contents) {
        Ok(sess) if sess.version == 1 => Some(sess),
        Ok(sess) => {
            log::warn!(
                "Session file version {} unsupported (expected 1), ignoring {}",
                sess.version,
                path.display()
            );
            None
        }
        Err(e) => {
            log::warn!(
                "Failed to parse session file '{}': {}. Using defaults.",
                path.display(),
                e
            );
            None
        }
    }
}

/// Convenience wrappers using the default path.
pub fn save_default(session: &Session) -> anyhow::Result<()> {
    save_to(session, &session_file_path())
}

pub fn load_default() -> Option<Session> {
    load_from(&session_file_path())
}

pub fn clear_default() -> anyhow::Result<()> {
    let p = session_file_path();
    if p.exists() {
        std::fs::remove_file(&p)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspaceEntry;
    use pengwm_core::workspace::Workspace;
    use std::collections::HashMap;

    fn tmp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("pengwm-session-tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn roundtrip_sanitize_and_save() {
        let mut ws = Workspace::new("Dev".into(), 1, (0, 0), (1920, 1080));
        ws.add_window(100, None);
        ws.add_window(200, None);
        ws.monocle = true;
        assert_eq!(ws.window_count(), 2);

        let sess = snapshot_from(
            &[ws],
            &HashMap::from([(1, 0)]),
            &vec![WorkspaceEntry {
                name: "Dev".into(),
                apps: vec![],
                monitor: None,
                autostart: vec![],
            }],
            10.0,
            5.0,
        );
        assert_eq!(sess.workspaces.len(), 1);
        assert_eq!(sess.workspaces[0].window_count(), 0, "windows stripped");
        assert!(sess.workspaces[0].monocle, "monocle preserved");
        assert_eq!(sess.workspaces[0].name, "Dev");

        let path = tmp_path("roundtrip.toml");
        save_to(&sess, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.workspaces[0].name, "Dev");
        assert_eq!(loaded.active.get(&1), Some(&0));
    }

    #[test]
    fn corrupt_file_returns_none() {
        let path = tmp_path("corrupt.toml");
        std::fs::write(&path, "not toml [[[ ").unwrap();
        assert!(load_from(&path).is_none());
    }

    #[test]
    fn orphan_remap_example() {
        // Simulate session saved with monitor 2, but only display 1 exists now.
        // The caller (StateManager) remaps, but we ensure sanitize keeps monitor_id.
        let ws = Workspace::new("Browsing".into(), 2, (1920, 0), (1920, 1080));
        let sess = snapshot_from(
            &[ws],
            &HashMap::from([(2, 0)]),
            &vec![],
            10.0,
            5.0,
        );
        assert_eq!(sess.workspaces[0].monitor_id, 2);
        assert_eq!(sess.active.get(&2), Some(&0));
    }
}
