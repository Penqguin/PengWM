use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use pengwm_core::workspace::Workspace;

mod string_key_map {
    use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S>(map: &BTreeMap<u32, usize>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let string_map: BTreeMap<String, usize> =
            map.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        string_map.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<u32, usize>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let string_map = BTreeMap::<String, usize>::deserialize(deserializer)?;
        string_map
            .into_iter()
            .map(|(k, v)| {
                k.parse::<u32>()
                    .map(|nk| (nk, v))
                    .map_err(de::Error::custom)
            })
            .collect()
    }
}

/// Versioned on-disk session snapshot. Stored at
/// `~/.local/share/pengwm/state.toml` (or `$XDG_STATE_HOME/pengwm/state.toml`).
///
/// Deliberately carries no routing `entries`: routing is a config-owned
/// birth decision (`Settings.workspaces`), and a snapshot that carried it
/// became a second source of truth — a session saved by an older binary
/// shadowed every later default/config change and re-saved itself on each
/// quit (the stale-entries fork). The session owns dynamic state only:
/// workspace topology, active seats, gaps, and the window placement
/// memory (below) that re-homes windows after a WM restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub workspaces: Vec<Workspace>,
    #[serde(with = "string_key_map")]
    pub active: BTreeMap<u32, usize>,
    /// Where each window lived at save time, matched at restore by durable
    /// identity (pid + bundle + title — `WindowPlacement::matches`), not by
    /// the ephemeral `WindowId`. Empty on a fresh init; old files load with
    /// an empty default. Consumed once per window, one ask at routing.
    #[serde(default)]
    pub placements: Vec<WindowPlacement>,
    pub gap_outer: f64,
    pub gap_inner: f64,
}

/// One window's remembered home: the workspace **name** (names are stable
/// across restore; indices are not) plus the identity evidence the next
/// boot can re-verify. `WindowId` is deliberately absent — it dies with the
/// daemon process. Bundle and title survive it, across a WM restart *and*
/// a reboot; the pid is deliberately absent too — it is app-level
/// identity, so an app's next window could wrongly inherit it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowPlacement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub workspace: String,
}

impl WindowPlacement {
    /// Does this record describe the window behind (`bundle`, `title`)?
    /// Every evidence pair both sides can supply must agree, and at least
    /// one window-level dimension must positively match — a record with no
    /// identity evidence is never a hit (it would match everything).
    /// Evidence one side cannot read does not veto: the boot where the
    /// bundle read times out still re-homes by title.
    pub fn matches(&self, bundle: Option<&str>, title: Option<&str>) -> bool {
        if self.title.is_some() && title.is_some() && self.title.as_deref() != title {
            return false;
        }
        if self.bundle.is_some() && bundle.is_some() && self.bundle.as_deref() != bundle {
            return false;
        }
        let title_hit = self.title.as_deref().is_some_and(|t| title == Some(t));
        let bundle_hit = self.bundle.as_deref().is_some_and(|b| bundle == Some(b));
        title_hit || bundle_hit
    }
}

impl Session {
    pub fn new(
        workspaces: Vec<Workspace>,
        active: BTreeMap<u32, usize>,
        placements: Vec<WindowPlacement>,
        gap_outer: f64,
        gap_inner: f64,
    ) -> Self {
        Self {
            version: 1,
            workspaces,
            active,
            placements,
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

/// Sanitize a workspace for persistence: keep name/monitor/geometry/preset
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
    // Preserve preset position; magnify never persists. Focus/tree reset.
    sanitized.preset_index = ws.preset_index;
    sanitized
}

/// Build a Session snapshot from the live `StateManager` fields, sanitizing
/// workspaces so no WindowIds are persisted (they die with the process;
/// `placements` carries the durable identity instead). Routing entries are
/// config-owned and never ride the snapshot (see the `Session` docs).
pub fn snapshot_from(
    workspaces: &[Workspace],
    active: &BTreeMap<u32, usize>,
    placements: Vec<WindowPlacement>,
    gap_outer: f64,
    gap_inner: f64,
) -> Session {
    let sanitized: Vec<Workspace> = workspaces.iter().map(sanitize_workspace).collect();
    Session::new(sanitized, active.clone(), placements, gap_outer, gap_inner)
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
    use pengwm_core::workspace::Workspace;
    use std::collections::BTreeMap;

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
        ws.toggle_magnify();
        assert_eq!(ws.window_count(), 2);

        let sess = snapshot_from(&[ws], &BTreeMap::from([(1, 0)]), vec![], 10.0, 5.0);
        assert_eq!(sess.workspaces.len(), 1);
        assert_eq!(sess.workspaces[0].window_count(), 0, "windows stripped");
        assert!(
            sess.workspaces[0].magnified.is_none(),
            "magnify never persists"
        );
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
    fn legacy_file_with_entries_still_loads() {
        // Pre-candidate-1 state.toml files carry `[[entries]]` sections (the
        // stale-entries fork). Unknown fields are ignored on deserialize, so
        // an untouched state file loads and the fork is discarded.
        let ws = Workspace::new("Dev".into(), 1, (0, 0), (1920, 1080));
        let sess = snapshot_from(&[ws], &BTreeMap::from([(1, 0)]), vec![], 10.0, 5.0);
        let mut toml_str = toml::to_string_pretty(&sess).unwrap();
        toml_str.push_str("\n[[entries]]\nname = \"Stale\"\n");
        let path = tmp_path("legacy.toml");
        std::fs::write(&path, toml_str).unwrap();
        let loaded = load_from(&path).expect("legacy session should load");
        assert_eq!(loaded.workspaces[0].name, "Dev");
        assert_eq!(loaded.gap_outer, 10.0);
    }

    #[test]
    fn orphan_remap_example() {
        // Simulate session saved with monitor 2, but only display 1 exists now.
        // The caller (StateManager) remaps, but we ensure sanitize keeps monitor_id.
        let ws = Workspace::new("Browsing".into(), 2, (1920, 0), (1920, 1080));
        let sess = snapshot_from(&[ws], &BTreeMap::from([(2, 0)]), vec![], 10.0, 5.0);
        assert_eq!(sess.workspaces[0].monitor_id, 2);
        assert_eq!(sess.active.get(&2), Some(&0));
    }

    #[test]
    fn placement_matches_by_every_evidence_pair_both_sides_supply() {
        let rec = WindowPlacement {
            bundle: Some("org.mozilla.firefox".into()),
            title: Some("PengWM — Development".into()),
            workspace: "Browsing".into(),
        };
        assert!(rec.matches(Some("org.mozilla.firefox"), Some("PengWM — Development")));
        // Different title: never a hit, even with an identical bundle.
        assert!(!rec.matches(Some("org.mozilla.firefox"), Some("Untitled")));
        // Different bundle: never a hit (two apps spawning identically-titled
        // windows must not steal each other's record).
        assert!(!rec.matches(Some("com.apple.Preview"), Some("PengWM — Development")));
        // Different title AND different bundle: inadmissible (the window was
        // recreated, its identity drifted).
        assert!(!rec.matches(Some("com.apple.Preview"), Some("Untitled")));
        // A record with no identity evidence matches nothing.
        let bare = WindowPlacement {
            bundle: None,
            title: None,
            workspace: "Browsing".into(),
        };
        assert!(!bare.matches(Some("org.mozilla.firefox"), Some("Something")));
        assert!(!bare.matches(None, None));
    }

    #[test]
    fn placement_holds_when_one_side_cannot_read() {
        // Evidence one side can't read must not veto: the boot where the
        // bundle read times out still re-homes by title.
        let rec = WindowPlacement {
            bundle: Some("org.mozilla.firefox".into()),
            title: Some("Untitled".into()),
            workspace: "Browsing".into(),
        };
        assert!(rec.matches(None, Some("Untitled")));
        // ... and by bundle when the title went away (retitled windows keep
        // their home; re-bundled ones don't deserve it).
        assert!(rec.matches(Some("org.mozilla.firefox"), None));
    }

    #[test]
    fn roundtrip_persists_placements() {
        let ws = Workspace::new("Dev".into(), 1, (0, 0), (1920, 1080));
        let placements = vec![WindowPlacement {
            bundle: Some("org.mozilla.firefox".into()),
            title: Some("PengWM — Development".into()),
            workspace: "Browsing".into(),
        }];
        let sess = snapshot_from(&[ws], &BTreeMap::from([(1, 0)]), placements, 10.0, 5.0);
        let path = tmp_path("placements-roundtrip.toml");
        save_to(&sess, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.placements.len(), 1);
        assert_eq!(loaded.placements[0].workspace, "Browsing");
        assert_eq!(
            loaded.placements[0].bundle.as_deref(),
            Some("org.mozilla.firefox")
        );
    }
}
