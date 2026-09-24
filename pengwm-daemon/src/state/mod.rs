use crate::adapter::OsAdapter;
use crate::bar_server::BarSender;
use crate::config::keybinds::KeybindConfig;
use crate::config::Settings;
use crate::event_loop::DaemonEvent;
use crate::prefix::{PrefixConfig, PrefixKey};
use pengwm_core::command::{BarMessage, BarState, BarWorkspace};
use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub mod bar;
pub mod bootstrap;
pub mod commands;
pub mod display;
pub mod drag;
pub mod hidden;
pub mod layout_cache;
pub mod lifecycle;
pub mod monitors_wake;
pub mod reconcile;
pub mod session;
pub mod store;
#[cfg(test)]
mod tests;
use self::bar::{BarReserve, ReloadAction};
use self::display::DisplaySet;
use self::drag::DragState;
use self::layout_cache::PinState;
use self::store::WindowStore;

pub struct StateManager {
    workspaces: Vec<Workspace>,
    displays: DisplaySet,
    frontmost_pid: Option<i32>,
    store: WindowStore,
    os: Box<dyn OsAdapter>,
    event_tx: mpsc::Sender<DaemonEvent>,
    gap_outer: f64,
    gap_inner: f64,
    keybinds: Arc<Mutex<KeybindConfig>>,
    prefix: Arc<Mutex<PrefixKey>>,
    /// Share of the split the first window takes in the `main-*` presets.
    main_ratio: f64,
    restricted_apps: Vec<String>,
    bar_sender: BarSender,
    bar: BarReserve,
    /// Pids whose windows are never managed by the WM — currently the spawned
    /// `pengwm-bar` process, whose window must not be tiled.
    excluded_pids: Vec<i32>,
    last_layout_rects: HashMap<WindowId, Rect>,
    /// Last rect successfully pushed to the OS per window (tiles and hides),
    /// with the write time. `apply_layout` skips windows already at their
    /// target so redundant layouts don't hammer the AX API — Firefox reflows
    /// on every write and visibly crawls under the repeat storm, while native
    /// apps shrug it off. Only updated on success so failed writes retry.
    /// Entries are invalidated by `on_window_moved` when the window is
    /// genuinely displaced (user drag, app move) so the next layout
    /// re-asserts — without this, drag snap-back would compare equal and
    /// wrongly skip.
    applied_rects: HashMap<WindowId, (Rect, Instant)>,
    /// Last time a `set_window_rect` failure was log-emitted per window.
    /// Live resizes make the OS reject size writes with `kAXErrorFailure`
    /// on every layout until the drag settles — without this, each retry
    /// logs again and the log fills with ERROR spam. Successful writes
    /// clear the entry so the next failure episode logs fresh.
    layout_fail_logged: HashMap<WindowId, Instant>,
    /// First time a tracked window failed its write AND the OS no longer
    /// listed it. A single poll miss is not death: post-wake / transient AX
    /// hiccups make `windows_for_pid` (used by both the element refresh and
    /// the gone-verify poll) come back empty while the window is still alive
    /// — the same WindowId reappears seconds later. Untrack only after the
    /// window stays missing past `GONE_GRACE`. Cleared on success, destroy,
    /// terminate and wake.
    gone_since: HashMap<WindowId, Instant>,
    /// Consecutive stable-pinned write failures per window (`layout_cache`
    /// policy): the writer reports "drift pinned" when consecutive readbacks
    /// stop moving, meaning further writes are futile (busy Firefox event
    /// loop). After `PIN_STRIKES` the layout skips writes for `PIN_BACKOFF`
    /// and retries on a timer instead of storming. Time-bounded, never
    /// permanent — a late-becoming-resizable window heals at most one
    /// backoff late. Cleared on success, target change, displace, destroy,
    /// terminate and wake.
    pin_state: HashMap<WindowId, PinState>,
    drag: DragState,
    /// Set when `Command::Quit` is handled; the event loop polls this and
    /// returns so the daemon process can exit.
    shutdown_requested: bool,
    /// Deadline until which `on_window_focused` will not mutate `DisplaySet::active`.
    /// Set on explicit workspace switches to prevent focus-notification feedback loops
    /// that drag windows along to the new workspace.
    switch_debounce_until: Option<Instant>,
    cached_hidden_strategy: Option<crate::config::HiddenStrategy>,
    focus_first_on_switch: bool,
    /// Last time `on_tick` swept `poll_windows_for_pid` for windows whose
    /// `WindowCreated` notification was missed (Firefox tear-offs / incognito:
    /// no AX notification, or a transient non-manageable subrole at creation).
    /// Throttled so the AX query storm doesn't run on every tick.
    last_window_sweep: Instant,
}

impl StateManager {
    pub fn new(
        event_tx: mpsc::Sender<DaemonEvent>,
        keybinds: Arc<Mutex<KeybindConfig>>,
        prefix: Arc<Mutex<PrefixKey>>,
        mut os: Box<dyn OsAdapter>,
        bar_sender: BarSender,
        bar_pid: Option<i32>,
        excluded_pids: Vec<i32>,
    ) -> Self {
        let display_infos = os.active_displays();
        let mut store = WindowStore::new();

        let settings = Settings::load();
        // Try session restore (topology + active + gaps) when enabled.
        // Skipped in `cargo test` so unit tests start from a deterministic
        // fresh state; session restore is exercised via `session::tests`.
        let maybe_session = if settings.restore_last_session && !cfg!(test) {
            session::load_default()
        } else {
            None
        };

        let primary_id = os.primary_display_id();
        let assembled = crate::state::bootstrap::assemble(
            &display_infos,
            primary_id,
            &settings,
            maybe_session.as_ref(),
        );
        let workspaces = assembled.workspaces;
        let displays = assembled.displays;
        let use_session = assembled.use_session;
        let entries = assembled.entries;
        let (gap_outer, gap_inner) = (assembled.gap_outer, assembled.gap_inner);

        if !use_session && !cfg!(test) {
            crate::state::bootstrap::maybe_autostart(&entries, &display_infos, use_session);
        }

        let frontmost_pid = os.frontmost_pid();

        for pid in os.running_app_pids() {
            os.attach_observer(pid);
            for window_id in os.poll_windows_for_pid(pid) {
                store.register(window_id, pid);
                let _ = event_tx.try_send(DaemonEvent::WindowCreated(window_id, pid));
            }
        }

        let bar_spawned = bar_pid.is_some();
        let bar = BarReserve::new(settings.bar.clone(), bar_spawned);
        let excluded_pids = bar_pid.into_iter().chain(excluded_pids).collect::<Vec<_>>();
        let mut state = Self {
            workspaces,
            displays,
            frontmost_pid,
            store,
            os,
            event_tx,
            gap_outer,
            gap_inner,
            keybinds,
            prefix,
            main_ratio: pengwm_core::workspace::clamp_main_ratio(settings.main_ratio),
            restricted_apps: settings.restricted_apps,
            bar_sender,
            bar,
            excluded_pids,
            last_layout_rects: HashMap::new(),
            applied_rects: HashMap::new(),
            layout_fail_logged: HashMap::new(),
            gone_since: HashMap::new(),
            pin_state: HashMap::new(),
            drag: DragState::new(),
            shutdown_requested: false,
            switch_debounce_until: None,
            cached_hidden_strategy: Some(settings.windows.hidden_strategy),
            focus_first_on_switch: settings.focus_first_on_switch,
            // Force an immediate background sweep on the first tick: login-time
            // Firefox is often still transient (not manageable / not resizable)
            // during the bootstrap poll, and `ns_workspace::observe` registers
            // after `new` returns — the first sweep + misplaced reconcile is
            // the startup second pass.
            last_window_sweep: Instant::now() - Duration::from_secs(5),
        };

        // Hide every workspace that isn't the active one for its monitor.
        // On fresh init that's all i != 0; on session restore it's the saved active set.
        let active_set: std::collections::HashSet<usize> =
            state.displays.active().values().copied().collect();
        for i in 0..state.workspaces.len() {
            if !active_set.contains(&i) {
                state.hide_workspace(i);
            }
        }

        state.apply_bar_reservation();
        state.bar_sender.send(if state.bar.is_visible() {
            BarMessage::Show
        } else {
            BarMessage::Hide
        });
        state.publish_bar_state();
        state
    }

    fn active_workspace_idx(&self) -> usize {
        self.displays.active_workspace_idx(
            &self.workspaces,
            self.store.all_pids(),
            self.frontmost_pid,
        )
    }

    /// True once `Command::Quit` has been handled; the event loop polls this
    /// to know when to stop running.
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    fn reload_config(&mut self) {
        log::info!("Reloading config...");
        let (updated_settings, updated_keybinds) = match crate::config::loader::load() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to reload config: {}. Keeping previous config.", e);
                return;
            }
        };
        {
            let mut keybinds = self.keybinds.lock().expect("keybind mutex poisoned");
            *keybinds = updated_keybinds;
            log::info!("Config reloaded ({} bindings)", keybinds.bindings.len());
        }
        self.gap_outer = updated_settings.gap_outer.max(0) as f64;
        self.gap_inner = updated_settings.gap_inner.max(0) as f64;
        self.main_ratio = pengwm_core::workspace::clamp_main_ratio(updated_settings.main_ratio);
        if let Ok(mut prefix) = self.prefix.lock() {
            prefix.set_config(PrefixConfig::parse_or_default(
                &updated_settings.prefix,
                updated_settings.prefix_timeout_ms,
            ));
        }
        self.displays.set_max_tiles(updated_settings.max_tiles);
        self.restricted_apps = updated_settings.restricted_apps;
        self.cached_hidden_strategy = Some(updated_settings.windows.hidden_strategy);
        self.focus_first_on_switch = updated_settings.focus_first_on_switch;
        self.displays
            .set_entries(if updated_settings.workspaces.is_empty() {
                crate::config::default_workspaces()
            } else {
                updated_settings.workspaces.clone()
            });
        let reload_action = self.bar.on_reload(updated_settings.bar.clone());
        self.apply_layout(self.active_workspace_idx());
        self.bar_sender.send(BarMessage::Reload);
        match reload_action {
            ReloadAction::NeedsRestart => {
                log::info!(
                    "bar.enabled flipped to true at runtime; restart the daemon to spawn pengwm-bar"
                );
            }
            ReloadAction::ShouldExit => {
                log::info!("bar.enabled flipped to false; exiting pengwm-bar");
                self.bar_sender.send(BarMessage::Exit);
                self.apply_bar_reservation();
                self.publish_bar_state();
                return;
            }
            ReloadAction::Reapply => {}
        }
        self.bar_sender.send(if self.bar.is_visible() {
            BarMessage::Show
        } else {
            BarMessage::Hide
        });
        self.apply_bar_reservation();
        self.publish_bar_state();
        log::info!(
            "Config reloaded (gaps: {}/{})",
            self.gap_outer,
            self.gap_inner
        );
    }

    fn hide_workspace(&mut self, workspace_idx: usize) {
        let ws = &self.workspaces[workspace_idx];
        let window_ids = ws.all_windows();
        if window_ids.is_empty() {
            return;
        }
        let placement = match self.windows_hidden_strategy() {
            crate::config::HiddenStrategy::BottomEdge => {
                pengwm_core::layout::HidePlacement::BottomEdge(pengwm_core::layout::hidden_rect(
                    ws.monitor_origin(),
                    ws.monitor_size(),
                ))
            }
            crate::config::HiddenStrategy::FarOffscreen => {
                pengwm_core::layout::HidePlacement::FarOffscreen
            }
        };
        let placements: HashMap<WindowId, pengwm_core::layout::HidePlacement> =
            window_ids.into_iter().map(|wid| (wid, placement)).collect();
        log::debug!(
            "hide_workspace idx={} mon={} strategy={:?} placement={:?} windows={:?}",
            workspace_idx,
            ws.monitor_id,
            self.windows_hidden_strategy(),
            placement,
            placements.keys()
        );
        self.os.hide_windows(&placements);
        self.seed_hidden_rect(placement.rect(), placements.keys().copied());
    }

    fn windows_hidden_strategy(&self) -> crate::config::HiddenStrategy {
        self.windows_config_hidden_strategy()
    }

    fn windows_config_hidden_strategy(&self) -> crate::config::HiddenStrategy {
        self.cached_hidden_strategy.unwrap_or_default()
    }

    /// Re-tile every window currently tracked as hidden back into its
    /// remembered (or routed) workspace and clear `HiddenTracker`. Must clear
    /// the tracker so future focus/move events are not ignored for now-visible
    /// windows.
    fn reveal_all(&mut self) {
        if self.store.hidden_is_empty() {
            return;
        }
        // Snapshot keys to avoid borrow conflict with &mut self in loop.
        let hidden_ids = self.store.keys();
        for wid in hidden_ids {
            // Skip if already re-tiled via earlier iteration
            if self.find_workspace_for_window(wid).is_some() {
                self.store.hidden_remove(wid);
                continue;
            }
            // Reuse on_window_shown which does take_hidden + add_window_to_workspace
            self.on_window_shown(wid);
        }
        // Ensure any remaining entries (e.g. unknown pid) are cleared
        if !self.store.hidden_is_empty() {
            for (_, _) in self.store.drain() {}
        }
        // Re-layout visible workspaces to ensure tiling clean
        let visible: Vec<usize> = self.displays.active().values().copied().collect();
        for idx in visible {
            if idx < self.workspaces.len() {
                self.apply_layout(idx);
            }
        }
    }

    #[cfg(test)]
    fn set_hidden_strategy_for_test(&mut self, s: crate::config::HiddenStrategy) {
        self.cached_hidden_strategy = Some(s);
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn set_focus_first_for_test(&mut self, v: bool) {
        self.focus_first_on_switch = v;
    }

    /// Global-coordinate rect of the bar strip on the primary display, or
    /// `None` when the bar is hidden, not spawned, or no display geometry is
    /// available. Delegates to `BarReserve` — the one place that knows the
    /// spawn gate (CONTEXT.md).
    fn bar_reserved_rect(&self) -> Option<Rect> {
        self.bar.reserved_rect(&*self.os)
    }

    /// Push the current bar strip geometry into every workspace (only
    /// workspaces on the primary display are reserved) and re-lay-out.
    fn apply_bar_reservation(&mut self) {
        let affected = self.bar.apply_reservation(&mut self.workspaces, &*self.os);
        for i in affected {
            self.apply_layout(i);
        }
    }

    /// Build a fresh `BarState` snapshot and broadcast it to the bar.
    fn publish_bar_state(&mut self) {
        let active_idx = self.active_workspace_idx();
        let active_monitor = self.workspaces[active_idx].monitor_id;
        let split_direction = self.workspaces[active_idx].focused_split_direction();

        let workspaces: Vec<BarWorkspace> = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(i, ws)| {
                let is_active = ws.monitor_id == active_monitor
                    && self
                        .displays
                        .active()
                        .get(&ws.monitor_id)
                        .map(|&idx| idx == i)
                        .unwrap_or(false);
                let windows = ws
                    .all_windows()
                    .into_iter()
                    .filter_map(|wid| self.store.pid_for(wid))
                    .map(|pid| {
                        self.os
                            .app_name(pid)
                            .or_else(|| self.os.app_bundle_id(pid))
                            .unwrap_or_else(|| "unknown".into())
                    })
                    .collect();
                BarWorkspace {
                    name: ws.name.clone(),
                    monitor_id: ws.monitor_id,
                    window_count: ws.window_count(),
                    active: is_active,
                    windows,
                }
            })
            .collect();

        self.bar_sender.send(BarMessage::State(BarState {
            workspaces,
            active_workspace: active_idx,
            split_direction,
            rect: self.bar_reserved_rect(),
        }));
    }
}
