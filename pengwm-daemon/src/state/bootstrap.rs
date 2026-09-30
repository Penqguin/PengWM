use crate::config::{default_workspaces, Settings, WorkspaceEntry};
use crate::state::display::DisplaySet;
use crate::state::session::Session;
use pengwm_core::workspace::Workspace;

use crate::adapter::DisplayInfo;

/// Result of pure workspace/display assembly. Caller (StateManager) owns
/// `Vec<Workspace>` mutation and `BarReserve`/`Router` creation; this just
/// decides which workspaces exist, where they are, and what gaps to use.
/// No file I/O, no process spawning, no `cfg(test)` — I/O seam is the
/// function argument.
pub struct Assembled {
    pub workspaces: Vec<Workspace>,
    pub displays: DisplaySet,
    pub gap_outer: f64,
    pub gap_inner: f64,
    pub entries: Vec<WorkspaceEntry>,
    pub use_session: bool,
}

/// Pure assembly: from display infos + settings + optional session, produce
/// the workspace vec, display set, gaps and the resolved entries. No `OsAdapter`
/// needed beyond the display infos slice (already queried by caller).
/// entries flows: Settings.workspaces -> DisplaySet.entries -> Session.entries
/// (Candidate 6: intentional clone, not worth Arc — update all three on new field).
pub fn assemble(
    display_infos: &[DisplayInfo],
    primary_id: u32,
    settings: &Settings,
    maybe_session: Option<&Session>,
) -> Assembled {
    let (entries, use_session) = if let Some(sess) = maybe_session {
        if !sess.workspaces.is_empty() {
            (sess.entries.clone(), true)
        } else {
            let e = if settings.workspaces.is_empty() {
                default_workspaces()
            } else {
                settings.workspaces.clone()
            };
            (e, false)
        }
    } else {
        let e = if settings.workspaces.is_empty() {
            default_workspaces()
        } else {
            settings.workspaces.clone()
        };
        (e, false)
    };

    let mut displays = DisplaySet::with_max_tiles(entries.clone(), settings.max_tiles);
    let mut workspaces = Vec::new();

    if use_session {
        let sess = maybe_session.unwrap();
        // Migrate pre-global-pool sessions: one tree per name (first wins),
        // orphans remapped to primary, geometry refreshed.
        let mut seen = std::collections::HashSet::new();
        let primary_info = display_infos.iter().find(|d| d.id == primary_id);
        let primary_origin = primary_info.map(|d| d.origin).unwrap_or((0, 0));
        let primary_size = primary_info.map(|d| d.size).unwrap_or((1920, 1080));
        for ws in &sess.workspaces {
            if !seen.insert(ws.name.clone()) {
                continue;
            }
            let mut ws = ws.clone();
            let exists = display_infos.iter().any(|d| d.id == ws.monitor_id);
            if !exists {
                ws.monitor_id = primary_id;
                ws.set_monitor_origin(primary_origin);
                ws.update_monitor_geometry(primary_origin, primary_size);
            } else if let Some(info) = display_infos.iter().find(|d| d.id == ws.monitor_id) {
                ws.update_monitor_geometry(info.origin, info.size);
            }
            workspaces.push(ws);
        }
        for (mon_id, idx) in &sess.active {
            if display_infos.iter().any(|d| d.id == *mon_id) && *idx < workspaces.len() {
                displays.active_mut().insert(*mon_id, *idx);
            }
        }
        for info in display_infos {
            if !displays.active().contains_key(&info.id) {
                if let Some(idx) = workspaces.iter().position(|ws| ws.monitor_id == info.id) {
                    displays.active_mut().insert(info.id, idx);
                } else if !workspaces.is_empty() {
                    displays.active_mut().insert(info.id, 0);
                }
            }
        }
        // Focused output: primary when live, else first active.
        if display_infos.iter().any(|d| d.id == primary_id) {
            displays.set_focused_output(primary_id);
        } else if let Some(&mon) = displays.active().keys().next() {
            displays.set_focused_output(mon);
        }
    } else {
        displays.init_workspaces(&mut workspaces, display_infos, primary_id);
    }

    // Headless fallback when no displays (tests)
    if workspaces.is_empty() {
        let pid = display_infos.first().map(|d| d.id).unwrap_or(1);
        let origin = display_infos.first().map(|d| d.origin).unwrap_or((0, 0));
        workspaces.push(Workspace::new("ws-1".into(), pid, origin, (1920, 1080)));
        displays.active_mut().insert(pid, 0);
    }

    let (gap_outer, gap_inner) = if use_session {
        let sess = maybe_session.unwrap();
        (sess.gap_outer, sess.gap_inner)
    } else {
        (
            settings.gap_outer.max(0) as f64,
            settings.gap_inner.max(0) as f64,
        )
    };

    Assembled {
        workspaces,
        displays,
        gap_outer,
        gap_inner,
        entries,
        use_session,
    }
}

/// Spawn autostart commands for fresh init (not session restore). One run per
/// global workspace regardless of its `monitor` hint (#4/Q16): the hint is
/// placement, not a spawn gate, so undocked launches stay reproducible.
/// No-op when `use_session` is true.
pub fn maybe_autostart(entries: &[WorkspaceEntry], use_session: bool) {
    if use_session {
        return;
    }
    // cfg(test) guard is now on the caller — this function is pure spawning
    // and can be tested by passing empty entries.
    for entry in entries {
        if entry.autostart.is_empty() {
            continue;
        }
        for cmd in &entry.autostart {
            log::info!("Autostart for workspace '{}': {}", entry.name, cmd);
            let _ = std::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .spawn()
                .map(|mut child| {
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                })
                .map_err(|e| {
                    log::warn!(
                        "Failed to autostart '{}' for workspace '{}': {}",
                        cmd,
                        entry.name,
                        e
                    );
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::DisplayInfo;
    use crate::config::Settings;

    fn disp(id: u32, origin: (i32, i32)) -> DisplayInfo {
        DisplayInfo {
            id,
            origin,
            size: (1920, 1080),
        }
    }

    #[test]
    fn assemble_fresh_init_creates_workspaces() {
        let displays = vec![disp(1, (0, 0))];
        let settings = Settings::default();
        let a = assemble(&displays, 1, &settings, None);
        assert!(!a.workspaces.is_empty());
        assert_eq!(a.displays.active().get(&1), Some(&0));
        assert!(!a.use_session);
    }

    #[test]
    fn assemble_headless_fallback() {
        let settings = Settings::default();
        let a = assemble(&[], 1, &settings, None);
        assert_eq!(a.workspaces.len(), 1);
        assert_eq!(a.workspaces[0].name, "ws-1");
    }

    #[test]
    fn assemble_with_session_restores() {
        let displays = vec![disp(1, (0, 0))];
        let mut ws = pengwm_core::workspace::Workspace::new("Dev".into(), 1, (0, 0), (1920, 1080));
        ws.monocle = true;
        let sess = Session::new(
            vec![ws],
            std::collections::BTreeMap::from([(1, 0)]),
            crate::config::default_workspaces(),
            15.0,
            7.0,
        );
        let settings = Settings::default();
        let a = assemble(&displays, 1, &settings, Some(&sess));
        assert!(a.use_session);
        assert_eq!(a.gap_outer, 15.0);
        assert!(a.workspaces[0].monocle);
    }

    #[test]
    fn assemble_session_orphan_remapped() {
        let displays = vec![disp(1, (0, 0))];
        let ws =
            pengwm_core::workspace::Workspace::new("Browsing".into(), 2, (1920, 0), (1920, 1080));
        let sess = Session::new(
            vec![ws],
            std::collections::BTreeMap::from([(2, 0)]),
            vec![],
            10.0,
            5.0,
        );
        let settings = Settings::default();
        let a = assemble(&displays, 1, &settings, Some(&sess));
        assert_eq!(a.workspaces[0].monitor_id, 1, "orphan remapped to primary");
    }
}
