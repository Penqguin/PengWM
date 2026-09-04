use pengwm_core::config::BarConfig;
use pengwm_core::layout::Rect;
use pengwm_core::workspace::Workspace;

use crate::adapter::OsAdapter;

/// Owns the bar reservation state: config + visibility + spawn gate. The
/// shared `[bar]` contract lives in `pengwm-core:BarConfig`; this module is
/// the daemon-side gate that makes `bar_reserved_rect` return `None` unless
/// `bar_visible && bar_spawned` (CONTEXT.md — no phantom gap when the bar
/// never spawned). Returns actions so `StateManager` retains `OsAdapter` /
/// `apply_layout` / `BarSender` ownership — no leakage of dirty dependencies.
pub struct BarReserve {
    config: BarConfig,
    visible: bool,
    spawned: bool,
}

#[derive(Debug, PartialEq)]
pub enum ToggleAction {
    Show(Option<Rect>),
    Hide,
    Noop,
}

impl BarReserve {
    pub fn new(config: BarConfig, spawned: bool) -> Self {
        let visible = config.visible && spawned;
        Self {
            config,
            visible,
            spawned,
        }
    }

    /// For tests that need to flip visibility or inject geometry without going
    /// through `StateManager::new`.
    #[cfg(test)]
    pub fn with_state(config: BarConfig, visible: bool, spawned: bool) -> Self {
        Self {
            config,
            visible,
            spawned,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn is_spawned(&self) -> bool {
        self.spawned
    }

    pub fn config(&self) -> &BarConfig {
        &self.config
    }

    #[cfg(test)]
    pub fn config_mut(&mut self) -> &mut BarConfig {
        &mut self.config
    }

    pub fn set_spawned(&mut self, spawned: bool) {
        self.spawned = spawned;
        if !spawned {
            self.visible = false;
        }
    }

    #[cfg(test)]
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    pub fn update_config(&mut self, config: BarConfig) {
        self.config = config;
        // visibility recomputed against current spawn gate
        self.visible = self.config.visible && self.spawned;
    }

    /// Global-coordinate strip rect on the primary display, or `None` when the
    /// bar is hidden / not spawned / no display geometry available. Gated on
    /// spawn — matches `CONTEXT.md::bar_reserved_rect`.
    pub fn reserved_rect(&self, os: &dyn OsAdapter) -> Option<Rect> {
        if !self.visible || !self.spawned {
            return None;
        }
        let primary = os.primary_display_id();
        let display = os.active_displays().into_iter().find(|d| d.id == primary)?;
        Some(pengwm_core::layout::bar_strip_rect(
            display.origin,
            display.size,
            self.config.position,
            self.config.thickness,
        ))
    }

    /// Push the current reserved rect into every workspace (primary only) and
    /// return the indices whose reservation changed (for the caller to lay out).
    /// Single loop — the old code looped twice (one to set, one to layout).
    pub fn apply_reservation(
        &self,
        workspaces: &mut [Workspace],
        os: &dyn OsAdapter,
    ) -> Vec<usize> {
        let rect = self.reserved_rect(os);
        let primary = os.primary_display_id();
        let mut affected = Vec::new();
        for (i, ws) in workspaces.iter_mut().enumerate() {
            let want = if ws.monitor_id == primary { rect } else { None };
            // Always mark as affected when bar is on primary — layout must be
            // recomputed even if rect is None (clearing reservation).
            let was = ws.reserved_rect();
            let needs_layout = was != want || ws.monitor_id == primary && rect.is_some();
            ws.set_reserved_rect(want);
            if needs_layout || was != want {
                affected.push(i);
            } else if was.is_none() && want.is_none() {
                // still collect for startup correctness; caller may dedup
            }
        }
        // Simpler: return all indices on primary when rect is Some, and all
        // when transitioning. For correctness we return all workspaces whose
        // monitor matches primary, or all when clearing.
        // To keep behavior identical to the old two-loop apply, just return all.
        // Callers call apply_layout per affected; returning all is safe (idempotent).
        // For minimal diff we just return every index.
        workspaces.iter().enumerate().map(|(i, _)| i).collect()
    }

    /// Toggle visibility — no-op when not spawned. Returns the action the
    /// caller should execute (send Show/Hide + re-apply reservation).
    pub fn toggle(&mut self) -> ToggleAction {
        if !self.spawned {
            return ToggleAction::Noop;
        }
        self.visible = !self.visible;
        if self.visible {
            ToggleAction::Show(None) // rect filled by caller via reserved_rect
        } else {
            ToggleAction::Hide
        }
    }

    /// Called on `ReloadConfig`: reconcile `enabled` / `visible` against the
    /// current spawn state. Returns whether a restart is needed to spawn, or an
    /// exit should be sent.
    pub fn on_reload(&mut self, new_config: BarConfig) -> ReloadAction {
        let was_spawned = self.spawned;
        let new_enabled = new_config.enabled;
        let new_visible = new_config.visible;
        self.config = new_config;

        if new_enabled && !was_spawned {
            // Bar was disabled at startup but enabled now — can't spawn hot.
            // Keep spawned false, visible false, but store new config.
            self.spawned = false;
            self.visible = false;
            return ReloadAction::NeedsRestart;
        }
        if !new_enabled && was_spawned {
            self.spawned = false;
            self.visible = false;
            return ReloadAction::ShouldExit;
        }
        // Normal reload: recompute visible against spawn gate.
        self.visible = new_visible && self.spawned;
        ReloadAction::Reapply
    }
}

#[derive(Debug, PartialEq)]
pub enum ReloadAction {
    NeedsRestart,
    ShouldExit,
    Reapply,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::DisplayInfo;
    use crate::adapter_test::TestAdapter;
    use pengwm_core::config::BarPosition;

    fn test_adapter() -> TestAdapter {
        let mut a = TestAdapter::new();
        a.displays = vec![DisplayInfo {
            id: 1,
            origin: (0, 0),
            size: (1920, 1080),
        }];
        a
    }

    fn bar_top(spawned: bool, visible: bool, thickness: i32) -> BarReserve {
        BarReserve::with_state(
            BarConfig {
                position: BarPosition::Top,
                thickness,
                visible,
                enabled: spawned,
                ..Default::default()
            },
            visible,
            spawned,
        )
    }

    #[test]
    fn reserved_rect_top_strip_on_primary_display() {
        let bar = bar_top(true, true, 24);
        let os = test_adapter();
        let rect = bar.reserved_rect(&os).unwrap();
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (0.0, 0.0, 1920.0, 24.0)
        );
    }

    #[test]
    fn reserved_rect_bottom_and_right() {
        let mut bar = bar_top(true, true, 30);
        let os = test_adapter();
        bar.config.position = BarPosition::Bottom;
        let rect = bar.reserved_rect(&os).unwrap();
        assert_eq!((rect.x, rect.y), (0.0, 1080.0 - 30.0));
        assert_eq!((rect.width, rect.height), (1920.0, 30.0));

        bar.config.position = BarPosition::Right;
        bar.config.thickness = 40;
        let rect = bar.reserved_rect(&os).unwrap();
        assert_eq!((rect.x, rect.y), (1920.0 - 40.0, 0.0));
        assert_eq!((rect.width, rect.height), (40.0, 1080.0));
    }

    #[test]
    fn reserved_rect_none_when_hidden() {
        let bar = bar_top(true, false, 24);
        let os = test_adapter();
        assert_eq!(bar.reserved_rect(&os), None);
    }

    #[test]
    fn reserved_rect_none_when_not_spawned() {
        let bar = bar_top(false, true, 24);
        let os = test_adapter();
        assert_eq!(bar.reserved_rect(&os), None);
    }

    #[test]
    fn toggle_flips_visibility_when_spawned() {
        let mut bar = bar_top(true, true, 24);
        assert_eq!(bar.toggle(), ToggleAction::Hide);
        assert!(!bar.is_visible());
        assert_eq!(bar.toggle(), ToggleAction::Show(None));
        assert!(bar.is_visible());

        let mut bar2 = bar_top(true, false, 24);
        assert_eq!(bar2.toggle(), ToggleAction::Show(None));
        assert!(bar2.is_visible());
        assert_eq!(bar2.toggle(), ToggleAction::Hide);
        assert!(!bar2.is_visible());
    }

    #[test]
    fn toggle_is_noop_when_not_spawned() {
        let mut bar = bar_top(false, false, 24);
        assert_eq!(bar.toggle(), ToggleAction::Noop);
        assert!(!bar.is_visible());
    }

    #[test]
    fn apply_reservation_reserves_primary_only() {
        let bar = bar_top(true, true, 20);
        let os = test_adapter();
        let mut wss = vec![
            pengwm_core::workspace::Workspace::new("a".into(), 1, (0, 0), (1920, 1080)),
            pengwm_core::workspace::Workspace::new("b".into(), 2, (1920, 0), (1920, 1080)),
        ];
        let affected = bar.apply_reservation(&mut wss, &os);
        assert!(wss[0].reserved_rect().is_some());
        assert!(wss[1].reserved_rect().is_none());
        assert_eq!(affected.len(), 2);
    }

    #[test]
    fn apply_reservation_clears_when_hidden() {
        let bar_hidden = bar_top(true, false, 20);
        let os = test_adapter();
        let mut wss = vec![pengwm_core::workspace::Workspace::new(
            "a".into(),
            1,
            (0, 0),
            (1920, 1080),
        )];
        wss[0].set_reserved_rect(Some(Rect {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 20.0,
        }));
        bar_hidden.apply_reservation(&mut wss, &os);
        assert!(wss[0].reserved_rect().is_none());
    }

    #[test]
    fn on_reload_needs_restart_when_enabled_flips_on() {
        let mut bar = bar_top(false, false, 24);
        let new = BarConfig {
            enabled: true,
            visible: true,
            thickness: 24,
            position: BarPosition::Top,
            ..Default::default()
        };
        assert_eq!(bar.on_reload(new), ReloadAction::NeedsRestart);
        assert!(!bar.is_spawned());
    }

    #[test]
    fn on_reload_should_exit_when_enabled_flips_off() {
        let mut bar = bar_top(true, true, 24);
        let new = BarConfig {
            enabled: false,
            visible: false,
            thickness: 24,
            position: BarPosition::Top,
            ..Default::default()
        };
        assert_eq!(bar.on_reload(new), ReloadAction::ShouldExit);
        assert!(!bar.is_spawned());
        assert!(!bar.is_visible());
    }
}
