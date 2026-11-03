//! Rebuild all derived state from scratch and re-route members.
//! Reserved for structural config changes or an explicit command —
//! never an automatic topology reaction (ADR-0006). Incremental
//! topology healing (DisplaySet::sync + re-parks) is the automatic
//! response; this path is the nuclear option.
//!
//! The seatbelt (live_placement) is preserved across a reset so that
//! ADR-0005's promise holds: a window the store lost comes back to
//! where it lives this session. Members re-route through the birth
//! handler, which consults the seatbelt first.
//!
//! Incremental topology repair (`DisplaySet::sync` + re-parks) patched
//! member state piecemeal, and every failure this session observed was
//! a stale derived corner the incremental path kept re-deriving: a CG
//! id change orphaning panels, a band ratcheted from a width-pin,
//! seats pointed at a monitor that no longer exists. Reset semantics
//! replace repair: on any actual topology change, rebuild every piece
//! of *derived* state exactly the way the constructor does — fresh
//! `assemble` (no session), fresh store, fresh write cache, bands
//! discarded — and re-route every *tracked member* through the same
//! `WindowCreated` handler boot feeds. The only survivor classes are
//! OS-side facts the OS owns: element caches (keyed by window id),
//! keybind/prefix state, the menubar contract, and autostart — a hot
//! re-seat must not re-launch apps.
//!
//! Deliberate cost: session-level invariants (popup parking, magnify
//! pins) do not survive a reset — members re-route as births (config
//! routing, then the seatbelt, then the active seat). Manual workspace
//! moves DO survive, via the preserved seatbelt (ADR-0006).

use std::time::{Duration, Instant};

use super::{DragState, LayoutWriteCache, StateManager, WindowStore};

/// Minimum spacing between resets. A single replug produces a burst of
/// MonitorAdded/Removed/Resized events; each one reaching
/// `run_topology_sync` while the topology is still settling would
/// otherwise reset repeatedly (and mid-un-plug, with zero displays,
/// could even mis-assemble). The cooldown is per *successful* reset:
/// a settled topology costs a no-op sync instead.
#[allow(dead_code)]
const RESET_COOLDOWN: Duration = Duration::from_millis(1500);

/// True once a reset just ran (so its callers can log one coordinated
/// line instead of five).
#[allow(dead_code)]
pub(super) struct ResetOutcome {
    pub members: usize,
    pub workspaces: usize,
}

impl StateManager {
    /// Rebuild all derived state from scratch and re-route members.
    /// No-op on an empty display set (mid-un-plug) or inside the
    /// cooldown — both log, neither touches state.
    #[allow(dead_code)]
    pub(in crate::state) fn wm_reset(&mut self) -> Option<ResetOutcome> {
        let now = Instant::now();
        if let Some(last) = self.last_wm_reset {
            if now.duration_since(last) < RESET_COOLDOWN {
                log::debug!("wm_reset suppressed inside cooldown ({} since last)", now.duration_since(last).as_millis());
                return None;
            }
        }
        let display_infos = self.os.active_displays();
        if display_infos.is_empty() {
            log::warn!("wm_reset skipped: no live displays (mid-un-plug?)");
            return None;
        }

        log::info!("WM reset: rebuilding topology, trees and seats from scratch");
        // Members snapshot: the one state class that survives — who is
        // being managed. Routing re-derives from config, exactly like a
        // fresh boot.
        let members: Vec<(pengwm_core::tree::WindowId, i32)> = self
            .store
            .all_window_pids()
            .iter()
            .map(|(wid, pid)| (*wid, *pid))
            .collect();

        // Every derivation restarts fresh. Bands are daemon-learned
        // guesses about what the OS refuses to report: a fresh display
        // set invalidates — and must not inherit — them.
        self.layout_cache = LayoutWriteCache::new();
        crate::macos::cg_display::prune_learned_top_bands(&[]);
        // The seatbelt is preserved (ADR-0006 / ADR-0005): a window
        // the store lost comes back to where it lives this session, even
        // across a structural reset. Members re-route through the birth
        // handler, which consults `live_placement` first.

        // The startup-shape decode (never quietly default — see
        // `config::loader::resolve`); keybinds are not re-read by a reset:
        // the live keybind Arc is the running truth.
        let (settings, _) = crate::config::loader::resolve();
        let primary = self.os.primary_display_id();
        let assembled = super::bootstrap::assemble(&display_infos, primary, &settings, None);
        self.workspaces = assembled.workspaces;
        self.displays = assembled.displays;
        self.gap_outer = assembled.gap_outer;
        self.gap_inner = assembled.gap_inner;
        super::set_popup_ratio(&mut self.workspaces, settings.windows.popup_ratio);
        self.main_ratio = pengwm_core::workspace::clamp_main_ratio(settings.main_ratio);
        self.store = WindowStore::new();
        // Members re-route as births: the same handler the boot poll
        // feeds, run synchronously (house pattern — `apply_layout` also
        // re-enters `on_window_destroyed` for its dead windows).
        let member_count = members.len();
        for (wid, pid) in &members {
            self.store.register(*wid, *pid);
            self.on_window_created(*wid, *pid);
        }
        // Constructor's tail: hide seats whose workspace is not visible
        // (members routed into hidden trees still live; they re-park).
        let active_set: std::collections::HashSet<usize> =
            self.displays.active().values().copied().collect();
        for i in 0..self.workspaces.len() {
            if !active_set.contains(&i) {
                self.hide_workspace(i);
            }
        }
        // Timers local to the old world: switch debounce and drag
        // tracking point at geometry that just got replaced.
        self.switch_debounce_until = None;
        self.drag = DragState::new();
        // Immediate background sweep, mirroring the constructor: the
        // reset happened inside an event dispatch; the first tick is
        // the opportunity for missed participants to re-attach.
        self.last_window_sweep = now - Duration::from_secs(5);
        self.last_wm_reset = Some(now);

        self.flash_hud();
        self.publish_bar_state();
        Some(ResetOutcome {
            members: member_count,
            workspaces: self.workspaces.len(),
        })
    }
}
