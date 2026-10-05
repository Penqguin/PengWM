use std::collections::HashMap;
use std::time::{Duration, Instant};

use pengwm_core::layout::{
    rects_close, rects_displaced, Rect, WriteOutcome, DISPLACE_GRACE, LAYOUT_EPSILON,
};
use pengwm_core::tree::WindowId;

/// What the caller must do after recording a failed write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AfterWrite {
    /// Nothing further: still tracked, retrying, or backed off.
    Keep,
    /// The window stayed gone past `GONE_GRACE`: untrack it via the
    /// normal destroyed path.
    Untrack,
}

/// Owns the layout-write funnel and the maps it reasons about: skip-if-unchanged
/// (the Firefox reflow storm), the post-write grace/epsilon that keeps our own
/// animation settling from tripping snap-back, the gone grace for transient AX
/// blackouts, the pin backoff for futile writes, and the hidden-rect seeding
/// that keeps switch-back from comparing equal to a stale tiled entry.
///
/// Two funnels, one owner: `plan_writes` decides a layout pass, `sweep_displaced`
/// decides the misplaced tick. Callers feed one cheap read per window and
/// execute what comes back — no caller threads `window_rect` through
/// `should_write` / `is_displaced` by hand anymore. `StateManager` retains the
/// workspace tree, routing, and the `OsAdapter` — this module owns every
/// write-policy map, so destroy, terminate, and wake collapse to one `forget` /
/// `clear_on_wake` each. The interface is the test surface: policy tests target
/// these methods directly, with no `StateManager` harness.
pub(super) struct LayoutWriteCache {
    /// Last rect successfully pushed to the OS per window (tiles and hides),
    /// with the write time. `should_write` skips windows already at their
    /// target so redundant layouts don't hammer the AX API — Firefox reflows
    /// on every write and visibly crawls under the repeat storm, while native
    /// apps shrug it off. Only updated on success so failed writes retry.
    /// Entries are invalidated by `note_displaced` / `is_displaced` when the
    /// window is genuinely displaced (user drag, app move) so the next layout
    /// re-asserts — without this, drag snap-back would compare equal and
    /// wrongly skip.
    applied_rects: HashMap<WindowId, (Rect, Instant)>,
    /// Last time a transient failure was log-emitted per window. Live
    /// resizes make the OS reject size writes on every layout until the drag
    /// settles — without this, each retry logs again and the log fills with
    /// spam. Successful writes clear the entry so the next failure episode
    /// logs fresh.
    layout_fail_logged: HashMap<WindowId, Instant>,
    /// First time a tracked window reported `Gone`. A single miss is not
    /// death: post-wake / transient AX hiccups empty the writer's listing
    /// for live windows — the same WindowId reappears seconds later.
    /// Untrack only after the window stays missing past `GONE_GRACE`.
    /// Cleared on success, destroy, terminate and wake.
    gone_since: HashMap<WindowId, Instant>,
    /// Consecutive Pinned reports per window: the writer saw readbacks stop
    /// moving, meaning further writes are futile (busy app event loop).
    /// After `PIN_STRIKES` the layout skips writes for `PIN_BACKOFF` and
    /// retries on a timer instead of storming. Time-bounded, never
    /// permanent — a late-becoming-resizable window heals at most one
    /// backoff late. Cleared on success, target change, destroy, terminate
    /// and wake — deliberately *not* on displace: see `invalidate`.
    pin_state: HashMap<WindowId, PinState>,
}

impl LayoutWriteCache {
    /// A window the OS stops listing is kept tracked for this long before
    /// the caller treats it as genuinely closed. Covers the post-wake
    /// AX blackout and transient refresh races (same WindowId reappearing
    /// seconds later); real closes arrive via the destroyed notification
    /// immediately and don't wait on this. Do NOT shrink this to cover
    /// "faster untracking": post-wake polling returns *empty listings for
    /// live apps* (see `monitors_wake::on_system_woke`), so a short grace
    /// untracks live windows and they stop being tiled entirely.
    const GONE_GRACE: Duration = Duration::from_secs(10);
    /// Consecutive Pinned reports before writes back off. Three strikes is
    /// ~3–6s of futility evidence: fast enough to matter, slow enough to
    /// ride out transient contention without throttling a window that is
    /// still making progress.
    const PIN_STRIKES: u32 = 3;
    /// How long a pinned window's writes are skipped before the next retry.
    /// Bounds the heal delay for a late-becoming-resizable window while
    /// cutting a chronic storm to a fraction of its write volume.
    const PIN_BACKOFF: Duration = Duration::from_secs(15);

    pub(super) fn new() -> Self {
        Self {
            applied_rects: HashMap::new(),
            layout_fail_logged: HashMap::new(),
            gone_since: HashMap::new(),
            pin_state: HashMap::new(),
        }
    }

    /// The one funnel for "should this window be written": skip-if-unchanged,
    /// read-before-write, and pin backoff. `actual` is the caller's single
    /// cheap AX read (reads don't reflow; writes do). Returns false when the
    /// write is provably redundant, already claimed, or backed off.
    pub(super) fn should_write(
        &mut self,
        window_id: WindowId,
        target: Rect,
        actual: Option<Rect>,
    ) -> bool {
        // Skip windows already at their target — redundant AX writes are
        // what makes Firefox crawl (reflow per write).
        if self.applied_rects.get(&window_id).map(|(r, _)| r) == Some(&target) {
            return false;
        }
        // Read-before-write: with no `applied_rects` entry (post-wake
        // clear, fresh tile) a single cheap AX read that already matches
        // the target claims the entry and skips the write. `None`
        // (unreadable/stale element) falls through to the write, which is
        // what refreshes the element.
        if !self.applied_rects.contains_key(&window_id) {
            if let Some(actual) = actual {
                if rects_close(actual, target, LAYOUT_EPSILON) {
                    self.record_success(window_id, target);
                    return false;
                }
            }
        }
        // Pinned backoff: a window whose writes provably do nothing skips
        // the write and retries on a timer. A changed target is a fresh
        // situation — drop the pin and write.
        let now = Instant::now();
        if let Some(pin) = self.pin_state.get(&window_id).copied() {
            if pin.target != target {
                self.pin_state.remove(&window_id);
            } else if self.pin_backoff_active(window_id, now) {
                log::debug!(
                    "layout_writer: window {} pinned, backing off write (retry in {:?})",
                    window_id,
                    Self::PIN_BACKOFF
                );
                return false;
            }
        }
        true
    }

    /// True while writes to this window are backed off: `PIN_STRIKES`
    /// consecutive futile attempts at the same target, the last one inside
    /// `PIN_BACKOFF`. One definition, two readers — `should_write` gates on
    /// it, and the misplaced sweep consults it so a pinned window stops
    /// re-triggering a whole-workspace re-layout on every tick.
    pub(super) fn pin_backoff_active(&self, window_id: WindowId, now: Instant) -> bool {
        self.pin_state.get(&window_id).is_some_and(|pin| {
            pin.strikes >= Self::PIN_STRIKES
                && now.duration_since(pin.last_attempt) < Self::PIN_BACKOFF
        })
    }

    /// The one planning funnel for a layout pass: given tiled targets and one
    /// cheap AX read per window, return the writes to execute. Runs every
    /// target through `should_write`, so skip-if-unchanged, read-before-write
    /// claiming, and pin backoff all apply in one place. Windows absent from
    /// `actuals` read as unreadable and fall through to the write, which is
    /// what refreshes a stale element.
    pub(super) fn plan_writes(
        &mut self,
        targets: &HashMap<WindowId, Rect>,
        actuals: &HashMap<WindowId, Option<Rect>>,
    ) -> Vec<(WindowId, Rect)> {
        targets
            .iter()
            .filter(|(window_id, target)| {
                let actual = actuals.get(window_id).copied().flatten();
                self.should_write(**window_id, **target, actual)
            })
            .map(|(&window_id, &target)| (window_id, target))
            .collect()
    }

    /// Record a landed write: claims the skip entry and clears every
    /// failure episode so the next one logs and retries fresh.
    pub(super) fn record_success(&mut self, window_id: WindowId, target: Rect) {
        self.applied_rects
            .insert(window_id, (target, Instant::now()));
        self.layout_fail_logged.remove(&window_id);
        self.gone_since.remove(&window_id);
        self.pin_state.remove(&window_id);
    }

    /// Record a failed write and report what the caller must do. `Ok` is
    /// accepted defensively (callers route it to `record_success`); every
    /// other outcome updates exactly one episode: pin strikes, gone grace,
    /// or the throttled transient log. Failed writes never claim the skip
    /// entry, so the next layout retries.
    pub(super) fn record_failure(
        &mut self,
        window_id: WindowId,
        target: Rect,
        outcome: WriteOutcome,
    ) -> AfterWrite {
        match outcome {
            WriteOutcome::Ok => {
                self.record_success(window_id, target);
                AfterWrite::Keep
            }
            // Pinned window: consecutive readbacks stopped moving, so
            // further writes are futile (busy app event loop). Count
            // strikes toward backoff instead of throttled-logging
            // every failure — the storm is the problem, not the log.
            // Debug per strike, warn once when backoff engages.
            WriteOutcome::Pinned { actual, .. } => {
                // A Pinned report carries a readback, so the element
                // answered: it is alive, whatever else is wrong. Clear any
                // gone grace, or a single stale miss from minutes ago
                // combines with one fresh miss to untrack a live window
                // instantly — `gone_since` records the *first* miss, and
                // only a landed write used to clear it.
                self.gone_since.remove(&window_id);
                let now = Instant::now();
                let strikes = match self.pin_state.get(&window_id) {
                    Some(pin) if pin.target == target => pin.strikes + 1,
                    _ => 1,
                };
                self.pin_state.insert(
                    window_id,
                    PinState {
                        strikes,
                        last_attempt: now,
                        target,
                    },
                );
                if strikes == Self::PIN_STRIKES {
                    log::warn!(
                        "layout_writer: window {} pinned at {:?} (target {:?}), backing off — retrying every {:?}",
                        window_id,
                        actual,
                        target,
                        Self::PIN_BACKOFF
                    );
                } else {
                    log::debug!(
                        "layout_writer: window {} pinned strike {}/{} (target {:?} actual {:?})",
                        window_id,
                        strikes,
                        Self::PIN_STRIKES,
                        target,
                        actual
                    );
                }
                AfterWrite::Keep
            }
            // Gone: the writer already refreshed + re-discovered and still
            // missed, so no second poll here — just the grace timer.
            // Post-wake / transient AX blackouts empty the writer's listing
            // for live windows, so only windows still gone after GONE_GRACE
            // report Untrack.
            WriteOutcome::Gone => {
                let now = Instant::now();
                match self.gone_since.get(&window_id) {
                    Some(first) if now.duration_since(*first) >= Self::GONE_GRACE => {
                        log::warn!(
                            "layout_writer: window {} still gone after {:?}, untracking",
                            window_id,
                            Self::GONE_GRACE,
                        );
                        AfterWrite::Untrack
                    }
                    Some(_) => {
                        log::debug!(
                            "layout_writer: window {} missing, within gone grace — keeping",
                            window_id,
                        );
                        AfterWrite::Keep
                    }
                    None => {
                        log::debug!(
                            "layout_writer: window {} missing, starting gone grace — keeping",
                            window_id,
                        );
                        self.gone_since.insert(window_id, now);
                        AfterWrite::Keep
                    }
                }
            }
            // Drift + transient contention: the write never landed.
            // Throttled log, retry on the next layout.
            WriteOutcome::Drift { target, actual } => {
                // Same reasoning as Pinned: a drift report means the
                // element answered a readback, so it is not gone.
                self.gone_since.remove(&window_id);
                self.log_transient(
                    window_id,
                    &format!(
                        "drift did not converge target {:?} actual {:?}",
                        target, actual
                    ),
                );
                AfterWrite::Keep
            }
            WriteOutcome::Transient(msg) => {
                self.log_transient(window_id, &msg);
                AfterWrite::Keep
            }
        }
    }

    /// Forget the skip entry so the next layout re-asserts the target
    /// (drag snap-back, app moves, the misplaced sweep).
    ///
    /// Deliberately leaves `pin_state` alone. Every caller is observing
    /// "this window is not where we put it" — which is the *expected*
    /// state of a pinned window, not new information. Clearing the strike
    /// count here reset it to 1 on every 2s sweep, so it could never reach
    /// `PIN_STRIKES` and the backoff never engaged for the one case it
    /// exists for: a busy app (Firefox after wake) refusing every write,
    /// stormed with a full 3-attempt rewrite every 2 seconds forever.
    /// The pin is dropped where it genuinely goes stale instead — a
    /// changed target (`should_write`), a landed write (`record_success`),
    /// destroy/terminate (`forget`), and wake (`clear_on_wake`).
    pub(super) fn invalidate(&mut self, window_id: WindowId) {
        self.applied_rects.remove(&window_id);
    }

    /// Drop every entry for a window (destroy / terminate paths).
    pub(super) fn forget(&mut self, window_id: WindowId) {
        self.applied_rects.remove(&window_id);
        self.layout_fail_logged.remove(&window_id);
        self.gone_since.remove(&window_id);
        self.pin_state.remove(&window_id);
    }

    /// Drop the whole cache: stale AX refs + moved displays mean every
    /// "already applied" entry is a lie after sleep. The gone grace
    /// restarts too — pre-sleep misses must not kill windows while
    /// post-wake AX is still blacked out — and so does the pin count, so
    /// post-wake writes aren't backoff-silenced.
    pub(super) fn clear_on_wake(&mut self) {
        self.applied_rects.clear();
        self.layout_fail_logged.clear();
        self.gone_since.clear();
        self.pin_state.clear();
    }

    /// If the window's actual rect genuinely disagrees with where we put it,
    /// invalidate so the next layout re-asserts (snap-back, app moves).
    /// Reads within the post-write grace window are our animation settling;
    /// reads within epsilon are jitter, not a drag. Full-rect, like the
    /// misplaced sweep: size drift counts too.
    ///
    /// `None` (unreadable) with a placed entry also invalidates: the window
    /// may be gone, and only a write attempt (→ `Gone` → grace) can tell —
    /// otherwise the skip entry strands it forever with no write ever
    /// observing the disappearance. `None` with no entry is a no-op.
    pub(super) fn note_displaced(&mut self, window_id: WindowId, actual: Option<Rect>) {
        let (target, written_at) = match self.applied_rects.get(&window_id) {
            Some(v) => *v,
            None => return,
        };
        let displaced = match actual {
            Some(r) => rects_displaced(r, target),
            None => true,
        };
        if displaced && Instant::now().duration_since(written_at) > DISPLACE_GRACE {
            log::debug!(
                "layout_writer: window {} displaced (actual {:?}), invalidating",
                window_id,
                actual
            );
            self.invalidate(window_id);
        }
    }

    /// Full-rect displacement check for the misplaced sweep: true when the
    /// OS rect genuinely disagrees with the tiled target. Recent writes are
    /// animation settling, not external drift. The caller invalidates and
    /// re-applies on true.
    pub(super) fn is_displaced(
        &self,
        window_id: WindowId,
        target: Rect,
        actual: Rect,
        now: Instant,
    ) -> bool {
        if !rects_displaced(actual, target) {
            return false;
        }
        if let Some((_, written_at)) = self.applied_rects.get(&window_id) {
            if now.duration_since(*written_at) < DISPLACE_GRACE {
                return false;
            }
        }
        true
    }

    /// The one sweep funnel for the misplaced tick: given tiled targets and
    /// one cheap AX read per window, invalidate genuinely displaced windows
    /// so the next layout re-asserts. `is_skipped` covers caller-owned skips
    /// (the active drag window, hidden windows) — pin backoff and
    /// grace/epsilon live here, next to `note_displaced`'s guards, instead of
    /// mirrored at a second call site. Unreadable windows are left alone:
    /// only a write attempt (→ `Gone` → grace) can judge them. Returns the
    /// invalidated windows so the caller can re-apply.
    pub(super) fn sweep_displaced(
        &mut self,
        targets: &HashMap<WindowId, Rect>,
        actuals: &HashMap<WindowId, Option<Rect>>,
        now: Instant,
        is_skipped: impl Fn(WindowId) -> bool,
    ) -> Vec<WindowId> {
        let mut invalidated = Vec::new();
        for (&window_id, &target) in targets {
            if is_skipped(window_id) {
                continue;
            }
            // Pinned and backed off: the app is provably refusing this
            // target, so it *will* read as displaced. Re-asserting here
            // would invalidate, re-layout the whole workspace and storm
            // the app on every tick — the backoff timer owns the retry
            // cadence instead. Skipped before the read: reads are cheap
            // but not free, and this one can only confirm what the pin
            // already recorded.
            if self.pin_backoff_active(window_id, now) {
                continue;
            }
            let actual = match actuals.get(&window_id).copied().flatten() {
                Some(r) => r,
                None => continue,
            };
            if !self.is_displaced(window_id, target, actual, now) {
                continue;
            }
            log::info!(
                "layout_writer: window {} displaced target ({:.0},{:.0} {}x{}) actual ({:.0},{:.0} {}x{}), invalidating",
                window_id,
                target.x,
                target.y,
                target.width,
                target.height,
                actual.x,
                actual.y,
                actual.width,
                actual.height
            );
            self.invalidate(window_id);
            invalidated.push(window_id);
        }
        invalidated
    }

    /// Record where hidden windows were put: a hidden rect never equals a
    /// future tile target, so this can't cause a wrongful skip — worst case
    /// one extra write. Without it, a window hidden after being tiled would
    /// compare equal to its stale tiled entry and never come back on
    /// switch-back.
    pub(super) fn seed_hidden(
        &mut self,
        rect: Rect,
        window_ids: impl IntoIterator<Item = WindowId>,
    ) {
        let written_at = Instant::now();
        for wid in window_ids {
            self.applied_rects.insert(wid, (rect, written_at));
        }
    }

    /// Throttled log for expected-transient failures: warn on the first
    /// failure per window per throttle window, debug on repeats. Repeats
    /// mean the next layout is still retrying the same contested write
    /// (e.g. an in-progress live resize), not new information.
    fn log_transient(&mut self, window_id: WindowId, msg: &str) {
        const FAIL_LOG_THROTTLE: Duration = Duration::from_secs(5);
        let now = Instant::now();
        let repeat = self
            .layout_fail_logged
            .get(&window_id)
            .is_some_and(|last| now.duration_since(*last) < FAIL_LOG_THROTTLE);
        if repeat {
            log::debug!(
                "layout_writer: write retry failed for window {}: {}",
                window_id,
                msg
            );
        } else {
            log::warn!(
                "layout_writer: write failed for window {}: {} (retrying)",
                window_id,
                msg
            );
            self.layout_fail_logged.insert(window_id, now);
        }
    }

    /// Drop the skip entry so the next layout takes the read-before-write
    /// path (mirrors the post-wake cleared cache).
    #[cfg(test)]
    pub(super) fn drop_applied_for_test(&mut self, window_id: WindowId) {
        self.applied_rects.remove(&window_id);
    }

    /// Age the skip entry past `DISPLACE_GRACE` so the next displacement
    /// check reads it as external, not animation settling.
    #[cfg(test)]
    pub(super) fn age_applied_for_test(&mut self, window_id: WindowId, age: Duration) {
        if let Some((rect, _)) = self.applied_rects.get(&window_id).copied() {
            self.applied_rects
                .insert(window_id, (rect, Instant::now() - age));
        }
    }

    /// Age the gone-grace entry past `GONE_GRACE` so the next miss untracks
    /// (mirrors a window staying closed).
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

    /// True when a transient failure was log-emitted for the window and no
    /// success has cleared it. Lets integration tests observe the throttle
    /// through the interface instead of the map.
    #[cfg(test)]
    pub(super) fn throttle_armed(&self, window_id: WindowId) -> bool {
        self.layout_fail_logged.contains_key(&window_id)
    }
}

/// Consecutive Pinned reports for one window and target, most recently at
/// `last_attempt`. Keyed per target so a changed target starts fresh.
#[derive(Clone, Copy)]
struct PinState {
    strikes: u32,
    last_attempt: Instant,
    target: Rect,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> Rect {
        Rect::new(0.0, 0.0, 960.0, 1040.0)
    }

    fn cache_with_placed_window() -> (LayoutWriteCache, Rect) {
        let mut cache = LayoutWriteCache::new();
        let t = target();
        cache.record_success(7, t);
        (cache, t)
    }

    #[test]
    fn skip_when_already_at_target() {
        let (mut cache, t) = cache_with_placed_window();
        assert!(!cache.should_write(7, t, None));
        assert!(!cache.should_write(7, t, Some(Rect::new(500.0, 500.0, 10.0, 10.0))));
    }

    #[test]
    fn read_before_write_claims_placed_window() {
        let mut cache = LayoutWriteCache::new();
        let t = target();
        // No entry, OS already at target: skip AND claim, so the next
        // layout skips without even needing the read.
        assert!(!cache.should_write(7, t, Some(t)));
        assert!(!cache.should_write(7, t, None));
    }

    #[test]
    fn read_before_write_miss_and_unreadable_fall_through() {
        let mut cache = LayoutWriteCache::new();
        let t = target();
        assert!(cache.should_write(7, t, Some(Rect::new(500.0, 500.0, 10.0, 10.0))));
        assert!(cache.should_write(7, t, None));
    }

    #[test]
    fn pin_strikes_back_off_then_heal() {
        let mut cache = LayoutWriteCache::new();
        let t = target();
        let pinned = |c: &LayoutWriteCache| c.pin_state.get(&7).map(|p| p.strikes).unwrap_or(0);
        for strike in 1..=3 {
            assert!(cache.should_write(7, t, None));
            assert_eq!(
                cache.record_failure(
                    7,
                    t,
                    WriteOutcome::Pinned {
                        target: t,
                        actual: Rect::new(500.0, 500.0, 10.0, 10.0),
                    }
                ),
                AfterWrite::Keep
            );
            assert_eq!(pinned(&cache), strike);
        }
        // Backoff engaged: no write.
        assert!(!cache.should_write(7, t, None));
        // Timer expires: retry.
        cache.age_pin_for_test(7, Duration::from_secs(30));
        assert!(cache.should_write(7, t, None));
        // Changed target: fresh situation, immediate write.
        let other = Rect::new(960.0, 0.0, 960.0, 1040.0);
        assert!(cache.should_write(7, other, None));
        // Healed: success clears the pin, skip resumes.
        cache.record_success(7, t);
        assert_eq!(pinned(&cache), 0);
        assert!(!cache.should_write(7, t, None));
    }

    #[test]
    fn invalidate_keeps_pin_evidence() {
        // The misplaced sweep invalidates a pinned window on every tick —
        // it reads as displaced *because* it is pinned. Clearing the strike
        // count there reset the counter to 1 forever, so it could never
        // reach PIN_STRIKES and the backoff never engaged.
        let mut cache = LayoutWriteCache::new();
        let t = target();
        let actual = Rect::new(500.0, 500.0, 10.0, 10.0);
        for strike in 1..=3 {
            assert!(cache.should_write(7, t, None));
            cache.record_failure(7, t, WriteOutcome::Pinned { target: t, actual });
            // What the sweep does on every tick.
            cache.invalidate(7);
            assert_eq!(cache.pin_state.get(&7).map(|p| p.strikes), Some(strike));
        }
        assert!(cache.pin_backoff_active(7, Instant::now()));
        assert!(
            !cache.should_write(7, t, None),
            "backoff must hold across invalidate"
        );
        // Still time-bounded, and a landed write still heals it.
        cache.age_pin_for_test(7, Duration::from_secs(30));
        assert!(!cache.pin_backoff_active(7, Instant::now()));
        assert!(cache.should_write(7, t, None));
        cache.record_success(7, t);
        assert!(!cache.pin_state.contains_key(&7));
    }

    #[test]
    fn gone_grace_keeps_then_untracks() {
        let mut cache = LayoutWriteCache::new();
        let t = target();
        assert_eq!(
            cache.record_failure(7, t, WriteOutcome::Gone),
            AfterWrite::Keep
        );
        assert_eq!(
            cache.record_failure(7, t, WriteOutcome::Gone),
            AfterWrite::Keep
        );
        cache.age_gone_for_test(7, Duration::from_secs(30));
        assert_eq!(
            cache.record_failure(7, t, WriteOutcome::Gone),
            AfterWrite::Untrack
        );
    }

    #[test]
    fn live_readback_clears_a_stale_gone_grace() {
        // `gone_since` records the *first* miss and used to survive until
        // a write landed. A window that missed once, then went minutes
        // without a successful write (it was at target, so writes were
        // skipped), got untracked by the very next single miss — the
        // grace had long since "expired". Any outcome carrying a readback
        // proves the element is alive and must reset it.
        let mut cache = LayoutWriteCache::new();
        let t = target();
        let actual = Rect::new(500.0, 500.0, 10.0, 10.0);
        assert_eq!(
            cache.record_failure(7, t, WriteOutcome::Gone),
            AfterWrite::Keep
        );
        // Minutes pass with the window alive but not written.
        cache.age_gone_for_test(7, Duration::from_secs(300));
        // A readback-carrying outcome proves it is alive.
        cache.record_failure(7, t, WriteOutcome::Pinned { target: t, actual });
        // The next miss starts a fresh grace instead of untracking.
        assert_eq!(
            cache.record_failure(7, t, WriteOutcome::Gone),
            AfterWrite::Keep,
            "a live readback must reset the gone grace"
        );
        // Drift resets it too.
        cache.age_gone_for_test(7, Duration::from_secs(300));
        cache.record_failure(7, t, WriteOutcome::Drift { target: t, actual });
        assert_eq!(
            cache.record_failure(7, t, WriteOutcome::Gone),
            AfterWrite::Keep
        );
        // A window that really is gone still untracks.
        cache.age_gone_for_test(7, Duration::from_secs(300));
        assert_eq!(
            cache.record_failure(7, t, WriteOutcome::Gone),
            AfterWrite::Untrack
        );
    }

    #[test]
    fn drift_and_transient_never_claim_and_arm_throttle() {
        let mut cache = LayoutWriteCache::new();
        let t = target();
        assert_eq!(
            cache.record_failure(
                7,
                t,
                WriteOutcome::Drift {
                    target: t,
                    actual: Rect::new(500.0, 500.0, 10.0, 10.0),
                }
            ),
            AfterWrite::Keep
        );
        assert!(cache.should_write(7, t, None));
        assert!(cache.throttle_armed(7));
        assert_eq!(
            cache.record_failure(
                7,
                t,
                WriteOutcome::Transient("kAXErrorFailure (live resize)".into())
            ),
            AfterWrite::Keep
        );
        assert!(cache.throttle_armed(7));
        // Success clears the episode.
        cache.record_success(7, t);
        assert!(!cache.throttle_armed(7));
        assert!(!cache.should_write(7, t, None));
    }

    #[test]
    fn displaced_outside_grace_invalidates() {
        let (mut cache, t) = cache_with_placed_window();
        let far = Rect::new(500.0, 500.0, 960.0, 1040.0);
        // Fresh write: settling, not a drag — even a far read doesn't invalidate.
        cache.note_displaced(7, Some(far));
        assert!(!cache.should_write(7, t, None));
        // Aged past grace: external move, rewrite.
        cache.age_applied_for_test(7, Duration::from_secs(5));
        cache.note_displaced(7, Some(far));
        assert!(cache.should_write(7, t, None));
        // Jitter inside epsilon never invalidates, however old.
        cache.record_success(7, t);
        cache.age_applied_for_test(7, Duration::from_secs(5));
        cache.note_displaced(7, Some(Rect::new(1.0, 1.0, 960.0, 1040.0)));
        assert!(!cache.should_write(7, t, None));
        // Unreadable element with a placed entry re-probes: the window may
        // be gone, and only a write attempt (→ Gone → grace) can tell.
        cache.note_displaced(7, None);
        assert!(cache.should_write(7, t, None));
        // Unreadable with no entry is a no-op (nothing claimed to keep).
        let mut fresh = LayoutWriteCache::new();
        fresh.note_displaced(7, None);
        assert!(fresh.should_write(7, t, None));
    }

    #[test]
    fn is_displaced_needs_full_rect_mismatch_past_grace() {
        let (mut cache, t) = cache_with_placed_window();
        let now = Instant::now();
        assert!(!cache.is_displaced(7, t, t, now));
        // Fresh write still settling: position drift doesn't count yet.
        assert!(!cache.is_displaced(7, t, Rect::new(500.0, 0.0, 960.0, 1040.0), now));
        // Past grace: position-only drift counts.
        cache.age_applied_for_test(7, Duration::from_secs(5));
        let now = Instant::now();
        assert!(cache.is_displaced(7, t, Rect::new(500.0, 0.0, 960.0, 1040.0), now));
        // ...but not while our own write is still settling.
        cache.record_success(7, t);
        assert!(!cache.is_displaced(7, t, Rect::new(500.0, 0.0, 960.0, 1040.0), Instant::now()));
        // Size-only drift counts once settled.
        cache.age_applied_for_test(7, Duration::from_secs(5));
        assert!(cache.is_displaced(7, t, Rect::new(0.0, 0.0, 800.0, 1040.0), Instant::now()));
    }

    #[test]
    fn seeded_hidden_rect_never_skips_a_tile() {
        let mut cache = LayoutWriteCache::new();
        let t = target();
        cache.seed_hidden(Rect::new(0.0, 1079.0, 1.0, 1.0), [7]);
        // Hidden rect != tile target: the next layout must write.
        assert!(cache.should_write(7, t, Some(Rect::new(0.0, 1079.0, 1.0, 1.0))));
    }

    #[test]
    fn plan_writes_batches_the_should_write_funnel() {
        let mut cache = LayoutWriteCache::new();
        let t = target();
        let other = Rect::new(960.0, 0.0, 960.0, 1040.0);
        let far = Rect::new(500.0, 500.0, 10.0, 10.0);
        // 7 already placed at target: skipped without a read.
        cache.record_success(7, t);
        let targets: HashMap<WindowId, Rect> = [(7, t), (8, other)].into_iter().collect();
        // 8 reads far from target: planned. 7 skipped even with a far read.
        let actuals: HashMap<WindowId, Option<Rect>> =
            [(7, Some(far)), (8, Some(far))].into_iter().collect();
        assert_eq!(cache.plan_writes(&targets, &actuals), vec![(8, other)]);
        // Missing read falls through to the write (stale-element refresh).
        let actuals: HashMap<WindowId, Option<Rect>> = [(8, None)].into_iter().collect();
        assert_eq!(cache.plan_writes(&targets, &actuals), vec![(8, other)]);
        // Read-before-write claims: OS already at target, no entry.
        let actuals: HashMap<WindowId, Option<Rect>> = [(8, Some(other))].into_iter().collect();
        assert!(cache.plan_writes(&targets, &actuals).is_empty());
    }

    #[test]
    fn sweep_displaced_invalidates_only_genuine_drift() {
        let mut cache = LayoutWriteCache::new();
        let t = target();
        let far = Rect::new(500.0, 500.0, 960.0, 1040.0);
        for wid in [7, 8, 9, 10] {
            cache.record_success(wid, t);
            cache.age_applied_for_test(wid, Duration::from_secs(5));
        }
        let targets: HashMap<WindowId, Rect> =
            [(7, t), (8, t), (9, t), (10, t)].into_iter().collect();
        let actuals: HashMap<WindowId, Option<Rect>> = [
            (7, Some(far)),  // genuinely displaced
            (8, Some(t)),    // at target
            (9, None),       // unreadable: left alone
            (10, Some(far)), // displaced but caller-skipped
        ]
        .into_iter()
        .collect();
        let invalidated =
            cache.sweep_displaced(&targets, &actuals, Instant::now(), |wid| wid == 10);
        assert_eq!(invalidated, vec![7]);
        // Only 7 rewrites now.
        assert!(cache.plan_writes(&targets, &actuals).contains(&(7, t)));
        assert!(!cache.plan_writes(&targets, &actuals).contains(&(8, t)));
    }

    #[test]
    fn sweep_displaced_respects_pin_backoff() {
        let mut cache = LayoutWriteCache::new();
        let t = target();
        let far = Rect::new(500.0, 500.0, 960.0, 1040.0);
        for _ in 1..=3 {
            cache.record_failure(
                7,
                t,
                WriteOutcome::Pinned {
                    target: t,
                    actual: far,
                },
            );
        }
        assert!(cache.pin_backoff_active(7, Instant::now()));
        let targets: HashMap<WindowId, Rect> = [(7, t)].into_iter().collect();
        let actuals: HashMap<WindowId, Option<Rect>> = [(7, Some(far))].into_iter().collect();
        // Reads as displaced, but the backoff owns the retry cadence.
        assert!(cache
            .sweep_displaced(&targets, &actuals, Instant::now(), |_| false)
            .is_empty());
    }

    #[test]
    fn forget_and_clear_on_wake_drop_everything() {
        let (mut cache, t) = cache_with_placed_window();
        cache.record_failure(7, t, WriteOutcome::Gone);
        cache.log_transient(7, "boom");
        cache.forget(7);
        assert!(cache.should_write(7, t, None));
        assert!(!cache.throttle_armed(7));

        cache.record_success(7, t);
        cache.record_failure(7, t, WriteOutcome::Gone);
        cache.clear_on_wake();
        // Post-wake: no skip entry (rewrite), no grace (fresh patience).
        assert!(cache.should_write(7, t, None));
        assert_eq!(
            cache.record_failure(7, t, WriteOutcome::Gone),
            AfterWrite::Keep
        );
    }
}
