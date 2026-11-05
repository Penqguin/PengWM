use pengwm_core::command::{BarMessage, PillCard};

use super::StateManager;

impl StateManager {
    pub(super) fn configure_pills(&mut self, enabled: bool, duration_ms: u64) {
        self.pill_enabled = enabled;
        self.pill_duration_ms = duration_ms.max(1);
        if !enabled {
            self.bar_sender.send(BarMessage::PillHide);
        }
    }

    pub(super) fn retain_workspace_pills(&self) {
        let display_ids = self
            .os
            .active_displays()
            .into_iter()
            .filter(|display| {
                self.displays.active().contains_key(&display.id)
                    && !self.space_limbo.contains(&display.id)
            })
            .map(|display| display.id)
            .collect();
        self.bar_sender.send(BarMessage::PillRetain { display_ids });
    }

    pub(super) fn flash_workspace_pills(&self) {
        if !self.pill_enabled {
            return;
        }
        let main_height = crate::macos::cg_display::raw_main_height();
        let active = self.displays.active();
        let focused = self.displays.focused_output();
        let mut cards = Vec::new();
        for display in self.os.active_displays() {
            if self.space_limbo.contains(&display.id) {
                continue;
            }
            let Some(&index) = active.get(&display.id) else {
                continue;
            };
            let Some(workspace) = self.workspaces.get(index) else {
                continue;
            };
            cards.push(PillCard {
                display_id: display.id,
                number: index + 1,
                name: workspace.name.clone(),
                focused: focused == Some(display.id),
                center: (
                    display.origin.0 as f64 + display.size.0 as f64 / 2.0,
                    main_height - (display.origin.1 as f64 + display.size.1 as f64 / 2.0),
                ),
            });
        }
        self.bar_sender.send(BarMessage::Pill {
            cards,
            duration_ms: self.pill_duration_ms.max(1),
        });
    }
}
