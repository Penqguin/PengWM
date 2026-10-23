use pengwm_core::tree::WindowId;

use super::session::WindowPlacement;
use super::StateManager;

/// Owns placement — the in-session seatbelt and the durable memory —
/// behind one ask (`where_should`) and one write (`note_where`). Two
/// stores, one interface:
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
/// - **The memory** (`placement_memory`): the last session's
///   (bundle, title → workspace name) records, loaded once at boot,
///   consumed hit-once at the ask. Restore input, never a live
///   membership owner (ADR-0003).
///
/// Membership itself stays single-sourced in the trees (ADR-0003) — both
/// stores are rediscovery/restore *input*: `where_should` answers, the
/// caller places through the normal routing funnel; config routing and
/// the active-workspace fallback stay the caller's job.
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
}

/// How long a dirty session waits before the tick drains it to disk.
/// Short enough that a crash loses seconds of placement, long enough
/// that a burst of changes (a wake resync re-homing several windows)
/// costs one write, not one per change.
pub(super) const SESSION_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

impl StateManager {
    /// The one placement ask: where should this window live? Three
    /// asks in precedence order, all inside this module:
    /// 1. **The seatbelt** — the most current fact about this window,
    ///    admitted only on a pid match (a recycled WindowId owned by a
    ///    different app inherits nothing).
    /// 2. **The durable memory** — restore-time, hit-once.
    /// 3. **The successor ask** — a re-created window's new WindowId
    ///    inherits the dead original's seatbelt home (see
    ///    `successor_home`).
    ///
    /// `None` when none knows; the caller falls through to config
    /// routing and the active workspace.
    pub(super) fn where_should(
        &mut self,
        window_id: WindowId,
        pid: i32,
        bundle: Option<&str>,
    ) -> Option<usize> {
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
        let title = self.os.window_title(window_id);
        let pos = self
            .placement_memory
            .iter()
            .position(|rec| rec.matches(bundle, title.as_deref()));
        if let Some(pos) = pos {
            let rec = self.placement_memory.remove(pos);
            if let Some(idx) = self
                .displays
                .index_of_name(&self.workspaces, &rec.workspace)
            {
                log::info!(
                    "Placement memory: window {} (bundle {:?}, title {:?}) re-homed to {} ({})",
                    window_id,
                    bundle,
                    title,
                    rec.workspace,
                    idx
                );
                return Some(idx);
            }
        }
        self.successor_home(window_id, pid, bundle, title.as_deref())
    }

    /// The successor ask: macOS keeps a CGWindowID for a window's
    /// lifetime *within a login session*, so a new WindowId carrying the
    /// same pid + durable identity while the recorded id no longer
    /// lists is that window's successor — the app-relaunch and
    /// window-recreation shapes, the one gap the WindowId key can't
    /// cover on its own. Exactly-one candidate admits the transfer (the
    /// stale record is consumed; the routing tail re-notes the home
    /// under the new id); an ambiguous match or none behaves as today.
    ///
    /// The discriminator is the pid's **current listing**: a live,
    /// listed sibling with the same identity is a sibling, not a
    /// successor. A `NoAnswer` listing judges nothing (blackout
    /// signature) — no transfer through a blackout. The poll runs only
    /// when a same-identity candidate exists, so ordinary routing pays
    /// nothing.
    fn successor_home(
        &mut self,
        window_id: WindowId,
        pid: i32,
        bundle: Option<&str>,
        title: Option<&str>,
    ) -> Option<usize> {
        let candidates: Vec<WindowId> = self
            .live_placement
            .iter()
            .filter(|(old_id, rec)| {
                **old_id != window_id
                    && rec.pid == pid
                    && super::session::identity_hit(
                        rec.bundle.as_deref(),
                        rec.title.as_deref(),
                        bundle,
                        title,
                    )
            })
            .map(|(old_id, _)| *old_id)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let poll = self.os.poll_windows_for_pid(pid);
        let listed = StateManager::listing_evidence(&poll)?;
        // The successor must itself be listed: a fresh id the app does
        // not (yet) list is too thin to admit a transfer — an app
        // mid-launch answers empty or partial, and guessing against a
        // poll that says nothing is how rediscovery mis-homes windows.
        if !listed.contains(&window_id) {
            return None;
        }
        let successors: Vec<WindowId> = candidates
            .into_iter()
            .filter(|old| !listed.contains(old))
            .collect();
        if successors.len() != 1 {
            return None;
        }
        let rec = self.live_placement.remove(&successors[0])?;
        if rec.idx >= self.workspaces.len() {
            return None;
        }
        log::info!(
            "Placement seatbelt: window {} is the successor of unlisted window {} (pid {}) — inherits workspace {}",
            window_id,
            successors[0],
            rec.pid,
            rec.idx
        );
        Some(rec.idx)
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
        if let Some(rec) = self.live_placement.get(&window_id) {
            if rec.pid == pid && rec.idx == idx && rec.bundle == bundle && rec.title == title {
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
            },
        );
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
    /// identity dimension — an identity-less record could never match
    /// at restore (`WindowPlacement::matches` needs a positive hit) and
    /// is skipped. Parked windows are here (noted at park time, while
    /// the identity was readable); popups are not (they re-place by
    /// monitor rect). Workspace **names** survive the reboot; indices
    /// do not. Reads no OS state — safe mid-blackout.
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

    #[cfg(test)]
    pub(super) fn seed_placement_memory_for_test(&mut self, records: Vec<WindowPlacement>) {
        self.placement_memory = records;
    }

    #[cfg(test)]
    pub(super) fn placement_memory_len_for_test(&self) -> usize {
        self.placement_memory.len()
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
