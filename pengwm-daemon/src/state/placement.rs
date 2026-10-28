use std::time::Instant;

use pengwm_core::tree::WindowId;

use super::session::WindowPlacement;
use super::StateManager;

/// Owns placement — the in-session seatbelt, its session projection and
/// persistence — behind one ask (`where_should`) and one write
/// (`note_where`):
///
/// - **The seatbelt** (`live_placement`): window → where it lives *right
///   now*, with its durable identity (bundle, title) captured at the
///   moment it got there. Noted at every placement change (routing,
///   manual move, park, reveal); consulted at rediscovery before
///   anything else, and projected at every session save (including the
///   throttled periodic one — the projection reads no OS state, so a
///   save is safe mid-blackout). Deliberately not evicted on window
///   destroy: a minimized window untracked by a listing misjudgment is
///   exactly the rediscovery this exists for, and a stale entry can only
///   mis-route if macOS recycles the WindowId for the same pid (same
///   app, benign). Entries die with the pid (`expire_pid`).
///
/// A new window never re-homes to a remembered workspace: routing for
/// births is seatbelt (same-window rediscovery only) → config routing
/// → the active workspace (ADR-0005). The session file still *records*
/// placements — the projection is diagnostic data and keeps the format
/// stable — but nothing consumes them at restore.
///
/// Membership itself stays single-sourced in the trees (ADR-0003) — the
/// seatbelt is rediscovery *input*: `where_should` answers, the caller
/// places through the normal routing funnel; config routing and the
/// active-workspace fallback stay the caller's job.
///
/// One identity read path: the durable identity is captured at note
/// time — the moment placement changes and the window is demonstrably
/// alive — not re-derived at quit time (the old read storm, which also
/// missed parked windows entirely: minimized-at-quit lost its home).
#[derive(Debug, Clone)]
pub(super) struct PlacementRecord {
    pub(super) pid: i32,
    pub(super) idx: usize,
    pub(super) bundle: Option<String>,
    pub(super) title: Option<String>,
    /// When the record was last confirmed as the window's current
    /// placement. The seatbelt is deliberately not evicted on window
    /// destroy, so a churn-heavy app (browsers, Electron) accretes one
    /// record per churned WindowId until the app dies — this timestamp
    /// is what the per-pid cap prunes by (newest K survive).
    pub(super) noted_at: Instant,
}

/// How many seatbelt records one pid may hold. Generous: a churn-heavy
/// app's rediscovery ask needs its *recent* records, and every entry
/// costs pid + idx + two identity strings.
const SEATBELT_CAP_PER_PID: usize = 8;

/// How long a dirty session waits before the tick drains it to disk.
/// Short enough that a crash loses seconds of placement, long enough
/// that a burst of changes (a wake resync re-homing several windows)
/// costs one write, not one per change.
pub(super) const SESSION_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

impl StateManager {
    /// The one placement ask: where should this window live? The
    /// **seatbelt** — the most current fact about this window,
    /// admitted only on a pid match (a recycled WindowId owned by a
    /// different app inherits nothing). This is rediscovery of an
    /// existing window, never a birth decision: a *new* window gets
    /// `None` and falls through to config routing and the active
    /// workspace (ADR-0005).
    pub(super) fn where_should(&mut self, window_id: WindowId, pid: i32) -> Option<usize> {
        if let Some(rec) = self.live_placement.get(&window_id) {
            if rec.pid == pid && rec.idx < self.workspaces.len() {
                log::info!(
                    "Placement seatbelt: rediscovered window {} pid {} returns to workspace {}",
                    window_id,
                    pid,
                    rec.idx
                );
                return Some(rec.idx);
            }
        }
        None
    }

    /// Seatbelt write, the one place a placement change is recorded.
    /// Captures the window's durable identity *now* — bundle and title
    /// are readable while the window is alive, including at park time
    /// (unlike later reads after a misjudgment). A no-op for windows
    /// the store can't name, no tree holds, or that sit in a popup set:
    /// popups re-place by monitor rect, the seatbelt records tree
    /// placement facts and never invents them.
    pub(super) fn note_where(&mut self, window_id: WindowId) {
        if self.is_popup_anywhere(window_id) {
            return;
        }
        let Some(pid) = self.store.pid_for(window_id) else {
            return;
        };
        let Some(idx) = self.find_workspace_of_any(window_id) else {
            return;
        };
        let bundle = self.os.app_bundle_id(pid);
        let title = self.os.window_title(window_id);
        let now = Instant::now();
        if let Some(rec) = self.live_placement.get_mut(&window_id) {
            if rec.pid == pid && rec.idx == idx && rec.bundle == bundle && rec.title == title {
                rec.noted_at = now;
                return;
            }
        }
        self.live_placement.insert(
            window_id,
            PlacementRecord {
                pid,
                idx,
                bundle,
                title,
                noted_at: now,
            },
        );
        // Cap per pid — the one boundedness rule the seatbelt map has.
        // Records die with the pid by design (a minimized window
        // untracked by a listing misjudgment must survive), so the only
        // unbounded growth is a never-quitting app that churns
        // WindowIds; the newest K per pid keep every legitimate
        // rediscovery ask.
        let mut same_pid: Vec<(WindowId, Instant)> = self
            .live_placement
            .iter()
            .filter(|(_, rec)| rec.pid == pid)
            .map(|(&wid, rec)| (wid, rec.noted_at))
            .collect();
        if same_pid.len() > SEATBELT_CAP_PER_PID {
            same_pid.sort_by_key(|&(_, at)| at);
            let excess = same_pid.len() - SEATBELT_CAP_PER_PID;
            for &(wid, _) in &same_pid[..excess] {
                if wid != window_id {
                    self.live_placement.remove(&wid);
                }
            }
        }
        self.session_dirty = true;
    }

    /// Seatbelt eviction for one pid: entries are pid-keyed by record
    /// and die with the pid, like the label cache — a recycled pid must
    /// not inherit the dead app's homes.
    pub(super) fn expire_pid(&mut self, pid: i32) {
        self.live_placement.retain(|_, rec| rec.pid != pid);
    }

    /// The durable projection: the session snapshot reads the seatbelt,
    /// not the trees. One record per noted window with at least one
    /// identity dimension. Parked windows are here (noted at park time,
    /// while the identity was readable); popups are not (they re-place
    /// by monitor rect). Workspace **names** survive the reboot; indices
    /// do not. Reads no OS state — safe mid-blackout. The records are
    /// diagnostic data since ADR-0005: saved, never consumed at
    /// restore.
    pub(super) fn capture_placements(&self) -> Vec<WindowPlacement> {
        self.live_placement
            .iter()
            .filter(|(_, rec)| rec.bundle.is_some() || rec.title.is_some())
            .map(|(wid, rec)| (wid, rec, self.workspaces.get(rec.idx)))
            .filter_map(|(wid, rec, ws)| {
                let ws = ws?;
                log::debug!(
                    "capture_placements: window {} → {} (bundle {:?}, title {:?})",
                    wid,
                    ws.name,
                    rec.bundle,
                    rec.title
                );
                Some(WindowPlacement {
                    bundle: rec.bundle.clone(),
                    title: rec.title.clone(),
                    workspace: ws.name.clone(),
                })
            })
            .collect()
    }

    /// The persistence funnel: snapshot + atomic save to `path`. The
    /// I/O seam is the path argument (bootstrap.rs's pattern) — no
    /// `cfg!(test)` guard: tests pass a tempdir and exercise the real
    /// round trip. Marks the projection clean on success.
    pub fn save_session(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        let sess = super::session::snapshot_from(
            &self.workspaces,
            self.displays.active(),
            self.capture_placements(),
            self.gap_outer,
            self.gap_inner,
        );
        let res = super::session::save_to(&sess, path);
        if res.is_ok() {
            self.session_dirty = false;
            self.last_session_save = std::time::Instant::now();
        }
        res
    }

    /// Tick hook: drain a dirty session at most once per
    /// `SESSION_SAVE_INTERVAL`. Errors are logged and the retry is
    /// deferred a full interval — a wedged disk must not re-fsync every
    /// tick; the next placement change re-dirties regardless.
    pub(super) fn save_session_if_dirty(&mut self, now: std::time::Instant) {
        if !self.session_dirty || now.duration_since(self.last_session_save) < SESSION_SAVE_INTERVAL
        {
            return;
        }
        let path = self.session_path.clone();
        if let Err(e) = self.save_session(&path) {
            log::warn!("Periodic session save failed: {}", e);
            self.last_session_save = now;
        }
    }

    /// Move the save clock past the drain interval, mirroring the wall
    /// clock a real interval would take.
    #[cfg(test)]
    pub(super) fn age_session_save_for_test(&mut self) {
        self.last_session_save =
            std::time::Instant::now() - SESSION_SAVE_INTERVAL - std::time::Duration::from_secs(1);
    }

    #[cfg(test)]
    pub(super) fn session_dirty_for_test(&self) -> bool {
        self.session_dirty
    }
}
