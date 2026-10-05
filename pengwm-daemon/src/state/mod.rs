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

pub mod bootstrap;
pub mod commands;
pub mod display;
pub mod drag;
pub mod layout_writer;
pub mod lifecycle;
pub mod monitors_wake;
pub mod reconcile;
pub mod session;
pub mod store;
#[cfg(test)]
mod tests;
use self::display::DisplaySet;
use self::drag::DragState;
use self::layout_writer::{AfterWrite, LayoutWriteCache};
use self::store::WindowStore;

/// Propagate the popup overlay ratio from settings to every workspace.
/// One definition for the two callers: construction and config reload.
fn set_popup_ratio(workspaces: &mut [Workspace], ratio: f64) {
    for ws in workspaces.iter_mut() {
        ws.popup_ratio = ratio;
    }
}

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
    /// Pids whose windows are never managed by the WM — the spawned
    /// `pengwm-menubar` child process, whose status item must not be tiled.
    excluded_pids: Vec<i32>,
    last_layout_rects: HashMap<WindowId, Rect>,
    /// Layout-write policy and the maps it reasons about (skip-if-unchanged,
    /// read-before-write, gone grace, pin backoff, hidden seeding). Owned by
    /// `LayoutWriteCache` — `StateManager` never touches the maps directly.
    layout_cache: LayoutWriteCache,
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
    /// Armed by `DaemonEvent::SystemWoke`, drained by `on_tick`. The wake
    /// notification arrives while the AX subsystem is still blacked out —
    /// polls come back empty and cached elements are stale — so the resync
    /// cannot run inline. See `monitors_wake::WakeResync`.
    wake_resync: Option<WakeResync>,
}

/// A wake resync waiting for the AX subsystem to come back.
///
/// `NSWorkspaceDidWake` fires before apps can answer accessibility
/// queries. A resync run at that moment polls every app and gets nothing,
/// so the element cache keeps its stale refs, and the layout writes that
/// follow all fail `kAXErrorInvalidUIElement` — starting a 10s gone-grace
/// death timer on every live window. `on_tick` retries until a poll comes
/// back with windows, or `DEADLINE` passes and it runs anyway.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WakeResync {
    /// When the wake notification arrived.
    pub(crate) since: Instant,
    /// When the last probe ran, so retries are paced.
    pub(crate) last_probe: Instant,
}

impl StateManager {
    pub fn new(
        event_tx: mpsc::Sender<DaemonEvent>,
        keybinds: Arc<Mutex<KeybindConfig>>,
        prefix: Arc<Mutex<PrefixKey>>,
        mut os: Box<dyn OsAdapter>,
        bar_sender: BarSender,
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
        let mut workspaces = workspaces;
        set_popup_ratio(&mut workspaces, settings.windows.popup_ratio);

        if !use_session && !cfg!(test) {
            crate::state::bootstrap::maybe_autostart(&entries, use_session);
        }

        let frontmost_pid = os.frontmost_pid();

        for pid in os.running_app_pids() {
            os.attach_observer(pid);
            for window_id in os.poll_windows_for_pid(pid) {
                store.register(window_id, pid);
                let _ = event_tx.try_send(DaemonEvent::WindowCreated(window_id, pid));
            }
        }

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
            excluded_pids,
            last_layout_rects: HashMap::new(),
            layout_cache: LayoutWriteCache::new(),
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
            wake_resync: None,
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
        set_popup_ratio(&mut self.workspaces, updated_settings.windows.popup_ratio);
        self.focus_first_on_switch = updated_settings.focus_first_on_switch;
        self.displays
            .set_entries(if updated_settings.workspaces.is_empty() {
                crate::config::default_workspaces()
            } else {
                updated_settings.workspaces.clone()
            });
        self.apply_layout(self.active_workspace_idx());
        self.publish_bar_state();
        log::info!(
            "Config reloaded (gaps: {}/{})",
            self.gap_outer,
            self.gap_inner
        );
    }

    fn hide_workspace(&mut self, workspace_idx: usize) {
        let ws = &self.workspaces[workspace_idx];
        let window_ids = ws.all_owned();
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
        self.layout_cache
            .seed_hidden(placement.rect(), placements.keys().copied());
    }

    fn windows_hidden_strategy(&self) -> crate::config::HiddenStrategy {
        self.windows_config_hidden_strategy()
    }

    fn windows_config_hidden_strategy(&self) -> crate::config::HiddenStrategy {
        self.cached_hidden_strategy.unwrap_or_default()
    }

    /// Re-tile every window currently tracked as hidden back into its
    /// remembered (or routed) workspace. The store owns the drain; routing
    /// needs displays + os, so the loop stays here. A retile publishes bar
    /// state, mirroring `on_window_shown`.
    fn reveal_all(&mut self) {
        let revealed = self.store.reveal_all();
        if revealed.is_empty() {
            return;
        }
        let mut retiled = false;
        for (wid, entry) in revealed {
            // Skip if already re-tiled via earlier iteration.
            if self.find_workspace_of_any(wid).is_some() {
                continue;
            }
            // Popups re-attach to their remembered workspace: no routing,
            // no capacity.
            if entry.popup {
                if entry.idx < self.workspaces.len() && self.workspaces[entry.idx].add_popup(wid) {
                    retiled = true;
                }
                continue;
            }
            let pid = match self.store.pid_for(wid) {
                Some(p) => p,
                None => continue,
            };
            let preferred = if entry.idx < self.workspaces.len() {
                entry.idx
            } else {
                self.displays
                    .routed_workspace_idx(pid, &self.workspaces, &*self.os)
                    .unwrap_or_else(|| self.active_workspace_idx())
            };
            if self.add_window_to_workspace(wid, pid, preferred).is_some() {
                retiled = true;
            }
        }
        // Re-layout visible workspaces to ensure tiling clean.
        let visible: Vec<usize> = self.displays.active().values().copied().collect();
        for idx in visible {
            if idx < self.workspaces.len() {
                self.apply_layout(idx);
            }
        }
        if retiled {
            self.publish_bar_state();
        }
    }

    #[cfg(test)]
    fn set_hidden_strategy_for_test(&mut self, s: crate::config::HiddenStrategy) {
        self.cached_hidden_strategy = Some(s);
    }

    #[cfg(test)]
    fn set_restricted_apps_for_test(&mut self, apps: Vec<String>) {
        self.restricted_apps = apps;
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn set_focus_first_for_test(&mut self, v: bool) {
        self.focus_first_on_switch = v;
    }

    /// Apply the tiled layout for one workspace: compute targets, take one
    /// cheap AX read per window (reads don't reflow; writes do), execute the
    /// writer's plan, and untrack windows proven gone via the normal destroyed
    /// path (removes from tree + store, re-layouts to fill the gap).
    pub(super) fn apply_layout(&mut self, workspace_idx: usize) {
        let rects = self.workspaces[workspace_idx].layout(self.gap_inner, self.gap_outer);
        // Drag hit-testing must never target a popup: popups are not tree
        // members, and swapping a tile with an overlay is meaningless. The
        // write plan below still sees the full map (popups ride it).
        let popup_ids = self.workspaces[workspace_idx].popup_ids();
        self.last_layout_rects = rects
            .iter()
            .filter(|(wid, _)| !popup_ids.contains(wid))
            .map(|(&wid, &rect)| (wid, rect))
            .collect();

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

        let actuals: HashMap<WindowId, Option<Rect>> = rects
            .keys()
            .map(|&window_id| (window_id, self.os.window_rect(window_id)))
            .collect();
        let mut dead = Vec::new();
        // `record_failure` routes `Ok` to `record_success` itself, so every
        // executed write commits through one call.
        for (window_id, target) in self.layout_cache.plan_writes(&rects, &actuals) {
            let outcome = self.os.set_window_rect(window_id, target);
            if self.layout_cache.record_failure(window_id, target, outcome) == AfterWrite::Untrack {
                dead.push(window_id);
            }
        }
        for window_id in dead {
            self.on_window_destroyed(window_id);
        }
    }

    /// Push the current UI state snapshot to the menubar.
    /// Primary-only bar over the global pool (#4/Q14): every workspace is
    /// listed, each output's visible workspace carries the active marker
    /// (`is_visible`, so two outputs → two markers); `active_workspace`
    /// is the focused one.
    fn publish_bar_state(&mut self) {
        let active_idx = self.active_workspace_idx();
        let split_direction = self.workspaces[active_idx].focused_split_direction();

        let workspaces: Vec<BarWorkspace> = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(i, ws)| {
                let is_active = self.displays.is_visible(i, &self.workspaces);
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
        }));
    }
}
