use std::time::{Duration, Instant};

use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;

use super::StateManager;

/// Owns the layout-write cache policy: skip-if-unchanged (the Firefox
/// reflow storm), the post-write grace/epsilon that keeps our own animation
/// settling from tripping snap-back, and the hidden-rect seeding that keeps
/// switch-back from comparing equal to a stale tiled entry. `StateManager`
/// retains the maps, the workspace tree and the `OsAdapter` — this module
/// only hides the policy. Everything is `pub(super)` so `mod.rs`,
/// commands.rs` and `tests.rs` keep calling the same interface.
///
/// Two reliability rules live here:
/// - Read-before-write: when there is no `applied_rects` entry (post-wake
///   clear, fresh tile), one cheap AX read that already matches the target
///   skips the write. Reads don't reflow; writes do (Firefox). This makes
///   wake resync cheap when windows haven't moved.
/// - Gone grace: a single poll miss never untracks. Post-wake / transient
///   AX hiccups empty `windows_for_pid` for live windows, so only windows
///   still missing after `GONE_GRACE` are dropped via the destroyed path.
/// - Pinned backoff: the writer reports "drift pinned" when consecutive
///   readbacks stop moving (busy app event loop ignoring writes). After
///   `PIN_STRIKES` consecutive pins the layout skips writes for `PIN_BACKOFF`
///   and retries on a timer — time-bounded, never permanent, so a
///   late-becoming-resizable window heals at most one backoff late.
impl StateManager {
    /// A window the OS stops listing is kept tracked for this long before
    /// `apply_layout` treats it as genuinely closed. Covers the post-wake
    /// AX blackout and transient refresh races (same WindowId reappearing
    /// seconds later); real closes arrive via the destroyed notification
    /// immediately and don't wait on this.
    const GONE_GRACE: Duration = Duration::from_secs(10);
    /// Consecutive "drift pinned" failures before writes back off. Three
    /// strikes is ~3–6s of futility evidence: fast enough to matter, slow
    /// enough to ride out transient contention without throttling a window
    /// that is still making progress.
    const PIN_STRIKES: u32 = 3;
    /// How long a pinned window's writes are skipped before the next retry.
    /// Bounds the heal delay for a late-becoming-resizable window while
    /// cutting a chronic storm to a fraction of its write volume.
    const PIN_BACKOFF: Duration = Duration::from_secs(15);

    pub(super) fn apply_layout(&mut self, workspace_idx: usize) {
        let rects = self.workspaces[workspace_idx].layout(self.gap_inner, self.gap_outer);
        self.last_layout_rects = rects.clone();

        log::debug!(
            "apply_layout ws={} gaps_in={} out={}:",
            workspace_idx,
            self.gap_inner,
            self.gap_outer
        );
        for (&window_id, rect) in &rects {
            log::debug!(
                "  win={} -> ({:.0},{:.0}) {}x{}",
                window_id,
                rect.x,
                rect.y,
                rect.width,
                rect.height
            );
        }

        let mut dead = Vec::new();
        for (&window_id, rect) in &rects {
            // Skip windows already at their target — redundant AX writes are
            // what makes Firefox crawl (reflow per write).
            if self.applied_rects.get(&window_id).map(|(r, _)| r) == Some(rect) {
                continue;
            }
            // Read-before-write: with no `applied_rects` entry (post-wake
            // clear, fresh tile) a single cheap AX read that already matches
            // the target skips the write. Reads don't reflow; writes do
            // (Firefox). This makes wake resync cheap when windows haven't
            // moved. `None` (unreadable/stale element) falls through to the
            // write, which is what refreshes the element.
            if !self.applied_rects.contains_key(&window_id) {
                if let Some(actual) = self.os.window_rect(window_id) {
                    if rects_close(actual, *rect, LAYOUT_EPSILON) {
                        self.applied_rects
                            .insert(window_id, (*rect, Instant::now()));
                        self.gone_since.remove(&window_id);
                        self.pin_state.remove(&window_id);
                        continue;
                    }
                }
            }
            // Pinned backoff: a window whose writes provably do nothing
            // (consecutive "drift pinned" failures) skips the write and
            // retries on a timer. Reads above still run, so a window the app
            // moved itself heals without writing. A changed target is a fresh
            // situation — drop the pin and write.
            let now = Instant::now();
            let pin = self.pin_state.get(&window_id).copied();
            match pin {
                Some(p) if p.target != *rect => {
                    self.pin_state.remove(&window_id);
                }
                Some(p)
                    if p.strikes >= Self::PIN_STRIKES
                        && now.duration_since(p.last_attempt) < Self::PIN_BACKOFF =>
                {
                    log::debug!(
                        "apply_layout: window {} pinned, backing off write (retry in {:?})",
                        window_id,
                        Self::PIN_BACKOFF
                    );
                    continue;
                }
                _ => {}
            }
            match self.os.set_window_rect(window_id, *rect) {
                Ok(()) => {
                    self.applied_rects
                        .insert(window_id, (*rect, Instant::now()));
                    self.layout_fail_logged.remove(&window_id);
                    self.gone_since.remove(&window_id);
                    self.pin_state.remove(&window_id);
                }
                Err(e) => {
                    let msg = e.to_string();
                    // Pinned window: consecutive readbacks stopped moving, so
                    // further writes are futile (busy app event loop). Count
                    // strikes toward backoff instead of throttled-logging
                    // every failure — the storm is the problem, not the log.
                    // Debug per strike, warn once when backoff engages.
                    if msg.contains("drift pinned") {
                        let now = Instant::now();
                        let strikes = match self.pin_state.get(&window_id) {
                            Some(pin) if pin.target == *rect => pin.strikes + 1,
                            _ => 1,
                        };
                        self.pin_state.insert(
                            window_id,
                            PinState {
                                strikes,
                                last_attempt: now,
                                target: *rect,
                            },
                        );
                        if strikes == Self::PIN_STRIKES {
                            log::warn!(
                                "apply_layout: window {} pinned (ignoring writes), backing off — retrying every {:?}",
                                window_id,
                                Self::PIN_BACKOFF
                            );
                        } else {
                            log::debug!(
                                "apply_layout: window {} pinned strike {}/{} ({})",
                                window_id,
                                strikes,
                                Self::PIN_STRIKES,
                                msg
                            );
                        }
                        continue;
                    }
                    // Permanent-gone signals: the adapter already refreshed +
                    // re-discovered and still failed. A single poll miss is
                    // NOT death — post-wake / transient AX blackouts empty
                    // `windows_for_pid` (used by both the refresh and this
                    // verify poll) for live windows — so record the first
                    // miss and only drop the window via the destroyed path
                    // once it stays missing past GONE_GRACE.
                    if msg.contains("kAXErrorInvalidUIElement")
                        || msg.contains("element not found in cache")
                    {
                        let still_exists = self
                            .store
                            .pid_for(window_id)
                            .map(|pid| self.os.poll_windows_for_pid(pid).contains(&window_id))
                            .unwrap_or(false);
                        if !still_exists {
                            let now = Instant::now();
                            match self.gone_since.get(&window_id) {
                                Some(first) if now.duration_since(*first) >= Self::GONE_GRACE => {
                                    log::warn!(
                                        "apply_layout: window {} still gone after {:?} ({}), untracking",
                                        window_id,
                                        Self::GONE_GRACE,
                                        msg
                                    );
                                    dead.push(window_id);
                                }
                                Some(_) => {
                                    log::debug!(
                                        "apply_layout: window {} missing ({}), within gone grace — keeping",
                                        window_id,
                                        msg
                                    );
                                }
                                None => {
                                    log::debug!(
                                        "apply_layout: window {} missing ({}), starting gone grace — keeping",
                                        window_id,
                                        msg
                                    );
                                    self.gone_since.insert(window_id, now);
                                }
                            }
                            continue;
                        }
                        // Listed but unwritable (refresh race): fall through
                        // to transient logging and retry next layout. Clear
                        // a stale grace entry — the window is back.
                        self.gone_since.remove(&window_id);
                    }
                    // Expected-transient AX contention, not a daemon bug.
                    // Live resizes reject size writes with kAXErrorFailure /
                    // kAXErrorCannotComplete until the drag settles, and a
                    // mid-flight element refresh surfaces InvalidUIElement
                    // while the window still exists. Warn once per window
                    // per throttle window, then debug — the write retries on
                    // the next layout anyway, so ERROR on every retry is spam.
                    if is_transient_ax_error(&msg) {
                        self.log_transient_layout_failure(window_id, &msg);
                    } else {
                        log::error!(
                            "apply_layout: set_window_rect failed for window {}: {}",
                            window_id,
                            e
                        );
                    }
                }
            }
        }
        // Untrack via the normal destroyed path (removes from tree + store,
        // re-layouts the visible workspace to fill the gap).
        for window_id in dead {
            self.on_window_destroyed(window_id);
        }
    }

    /// Throttled log for expected-transient layout failures: warn on the
    /// first failure per window per throttle window, debug on repeats.
    /// Repeats mean the next layout is still retrying the same contested
    /// write (e.g. an in-progress live resize), not new information.
    fn log_transient_layout_failure(&mut self, window_id: WindowId, msg: &str) {
        const FAIL_LOG_THROTTLE: Duration = Duration::from_secs(5);
        let now = Instant::now();
        let repeat = self
            .layout_fail_logged
            .get(&window_id)
            .is_some_and(|last| now.duration_since(*last) < FAIL_LOG_THROTTLE);
        if repeat {
            log::debug!(
                "apply_layout: set_window_rect retry failed for window {}: {}",
                window_id,
                msg
            );
        } else {
            log::warn!(
                "apply_layout: set_window_rect failed for window {}: {} (retrying)",
                window_id,
                msg
            );
            self.layout_fail_logged.insert(window_id, now);
        }
    }

    /// If the window is genuinely displaced from where we put it, forget
    /// the applied entry so the next layout re-asserts (snap-back, app
    /// moves). Two guards keep our own writes from tripping this:
    /// moves within the post-write grace window are our animation
    /// settling, and moves within a few px are jitter, not a drag.
    pub(super) fn note_displaced(&mut self, window_id: WindowId, x: f64, y: f64) {
        const MOVE_GRACE: Duration = Duration::from_millis(500);
        const MOVE_EPSILON: f64 = 8.0;
        if let Some((target, written_at)) = self.applied_rects.get(&window_id) {
            let now = Instant::now();
            let displaced =
                (x - target.x).abs() > MOVE_EPSILON || (y - target.y).abs() > MOVE_EPSILON;
            if displaced && now.duration_since(*written_at) > MOVE_GRACE {
                log::debug!(
                    "on_window_moved: window {} displaced to ({:.0},{:.0}), invalidating applied rect",
                    window_id,
                    x,
                    y
                );
                self.applied_rects.remove(&window_id);
                // Fresh situation for the pin count too: a moved window gets
                // prompt writes again, not backoff silence.
                self.pin_state.remove(&window_id);
            }
        }
    }

    /// Record where hidden windows were put: a hidden rect never equals a
    /// future tile target, so this can't cause a wrongful skip — worst case
    /// one extra write. Without it, a window hidden after being tiled would
    /// compare equal to its stale tiled entry and never come back on
    /// switch-back.
    pub(super) fn seed_hidden_rect(
        &mut self,
        rect: Rect,
        window_ids: impl IntoIterator<Item = WindowId>,
    ) {
        let written_at = Instant::now();
        for wid in window_ids {
            self.applied_rects.insert(wid, (rect, written_at));
        }
    }

    #[cfg(test)]
    pub(super) fn age_applied_for_test(&mut self, window_id: WindowId, age: Duration) {
        if let Some((rect, _)) = self.applied_rects.get(&window_id).copied() {
            self.applied_rects
                .insert(window_id, (rect, Instant::now() - age));
        }
    }

    /// Drop the skip-if-unchanged entry so the next layout takes the
    /// read-before-write path (mirrors the post-wake cleared cache).
    #[cfg(test)]
    pub(super) fn drop_applied_for_test(&mut self, window_id: WindowId) {
        self.applied_rects.remove(&window_id);
    }

    /// Age the gone-grace entry past `GONE_GRACE` so the next missing poll
    /// untracks (mirrors a window staying closed).
    #[cfg(test)]
    pub(super) fn age_gone_for_test(&mut self, window_id: WindowId, age: Duration) {
        if self.gone_since.contains_key(&window_id) {
            self.gone_since.insert(window_id, Instant::now() - age);
        }
    }

    /// Age the pin entry past `PIN_BACKOFF` so the next layout retries a
    /// backed-off window (mirrors the backoff timer expiring).
    #[cfg(test)]
    pub(super) fn age_pin_for_test(&mut self, window_id: WindowId, age: Duration) {
        if let Some(pin) = self.pin_state.get(&window_id).copied() {
            self.pin_state.insert(
                window_id,
                PinState {
                    last_attempt: Instant::now() - age,
                    ..pin
                },
            );
        }
    }
}

/// Consecutive stable-pinned failures for one window and target: the writer
/// reported "drift pinned" (readbacks stopped moving) `strikes` times in a
/// row for `target`, most recently at `last_attempt`. Keyed per target so a
/// changed target starts fresh.
#[derive(Clone, Copy)]
pub(super) struct PinState {
    strikes: u32,
    last_attempt: Instant,
    target: Rect,
}

/// Tolerance for "already at target" comparisons: matches the 8px the
/// misplaced sweep (`reconcile.rs`) and drag settle (`layout_cache.rs`)
/// use. Anything within this is "at target" everywhere in the system, so
/// accepting it here can't cause a wrongful permanent skip — and the AX
/// writer (`ax_element.rs`) uses the same value so a window it accepts
/// won't be flagged misplaced by the sweep.
const LAYOUT_EPSILON: f64 = 8.0;

/// Pure epsilon comparison for the read-before-write skip. Mirrors
/// `ax_element::rects_close` — keep the two in sync.
fn rects_close(a: Rect, b: Rect, eps: f64) -> bool {
    (a.x - b.x).abs() <= eps
        && (a.y - b.y).abs() <= eps
        && (a.width - b.width).abs() <= eps
        && (a.height - b.height).abs() <= eps
}

/// AX failures that are expected while a window is being live-resized or is
/// mid-flight through an element refresh — the write retries on the next
/// layout, so these are throttled warnings, not errors. Anything else (e.g.
/// permission loss) stays an error. Gone-window signals are classified by the
/// caller via the OS listing check before reaching here.
fn is_transient_ax_error(msg: &str) -> bool {
    // Size writes rejected while the user holds the resize handle, or while
    // the app clamps the size (min/max, fullscreen): the next layout retries.
    msg.contains("kAXErrorFailure")
        || msg.contains("kAXErrorCannotComplete")
        // Stale element for a window the OS still lists: the refresh race,
        // not a close. (Truly gone windows are untracked by the caller.)
        || msg.contains("kAXErrorInvalidUIElement")
        || msg.contains("element not found in cache")
        // Persistent position/size drift (Firefox frame-vs-content shift,
        // login-time not-yet-resizable windows): the write never landed so
        // `applied_rects` must not record success. Retry on next layout.
        || msg.contains("drift did not converge")
        || msg.contains("drift pinned")
        || msg.contains("drift attempt")
}

#[cfg(test)]
mod tests {
    use super::is_transient_ax_error;

    #[test]
    fn classifies_live_resize_contention_as_transient() {
        assert!(is_transient_ax_error(
            "AXUIElementSetAttributeValue size error: kAXErrorFailure"
        ));
        assert!(is_transient_ax_error(
            "AXUIElementSetAttributeValue size error: kAXErrorCannotComplete"
        ));
        assert!(is_transient_ax_error(
            "AXUIElementSetAttributeValue position error: kAXErrorInvalidUIElement"
        ));
        assert!(is_transient_ax_error(
            "element not found in cache for window 17912"
        ));
        assert!(is_transient_ax_error(
            "set_window_rect drift did not converge target Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 } actual Rect { x: 10.0, y: 10.0, width: 100.0, height: 100.0 }"
        ));
        assert!(is_transient_ax_error(
            "set_window_rect drift pinned target Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 } actual Rect { x: 10.0, y: 10.0, width: 100.0, height: 100.0 } (stable across attempts)"
        ));
    }

    #[test]
    fn unexpected_errors_stay_errors() {
        assert!(!is_transient_ax_error("AXIsProcessTrusted() == false"));
        assert!(!is_transient_ax_error("some unknown io failure"));
    }
}
