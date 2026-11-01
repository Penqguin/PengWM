//! The transient workspace HUD planner (Spec 0001) — the pure side of
//! the flash: which cards the pills display, built from the current
//! snapshot, plus the deadline bookkeeping the tick drains.
//!
//! Execution lives behind `OsAdapter::show_hud` / `hide_hud` (the
//! macOS side is `macos::hud::HudPanels`); triggers live with the
//! commands and the event tap. This module is what tests drive: cards
//! include per-monitor workspace identity, the focused monitor, and
//! the prefix-armed marker.

use std::time::{Duration, Instant};

use crate::adapter::HudCard;

use super::StateManager;

impl StateManager {
    /// Build one card per monitor (left-to-right, top-to-bottom) — every
    /// live display appears, even a display showing nothing (a "—" keeps
    /// every monitor in the reader's mental map). The display snapshot is
    /// fetched once by the caller and passed in: the plan is a pure
    /// function of its inputs, like `plan_switch` (`commands.rs`).
    pub(in crate::state) fn hud_cards(
        &self,
        now: Instant,
        displays: &[crate::adapter::DisplayInfo],
    ) -> Vec<HudCard> {
        let prefix_armed = self
            .prefix
            .lock()
            .map(|prefix| prefix.is_armed(now))
            .unwrap_or(false);
        let focused = self.displays.focused_output();
        let active = self.displays.active();
        let mut displays: Vec<_> = displays.to_vec();
        // Deterministic reading order: display position, not CG's
        // nondeterministic enumeration order. Left to right, then top.
        displays.sort_by_key(|d| (d.origin.0, d.origin.1));
        displays
            .into_iter()
            .map(|d| {
                let (number, name) = match active.get(&d.id) {
                    Some(&idx) if idx < self.workspaces.len() => {
                        (idx + 1, self.workspaces[idx].name.clone())
                    }
                    _ => (0, "—".to_string()),
                };
                HudCard {
                    display_id: d.id,
                    number,
                    name,
                    focused: focused == Some(d.id),
                    prefix_armed,
                }
            })
            .collect()
    }

    /// The flash: snapshot the cards, request a draw, (re)arm the hide
    /// deadline. Re-flashing while visible is exactly the wanted
    /// behavior — the pills update and stay up.
    pub(in crate::state) fn flash_hud(&mut self) {
        if !self.hud_enabled {
            return;
        }
        let now = Instant::now();
        self.hud_hide_at = Some(now + self.hud_duration());
        let displays = self.os.active_displays();
        self.os.show_hud(&self.hud_cards(now, &displays));
    }

    /// Re-show the HUD (now with the armed marker) and let the tick's
    /// deadline logic pin it while the chord stays armed.
    pub fn on_prefix_armed(&mut self) {
        self.flash_hud();
    }

    /// The tick's HUD drain: while the prefix is armed the pills stay
    /// (deadline keeps rolling); once it passes and the deadline runs
    /// out, the pills hide. A disabled HUD hides immediately — the kill
    /// switch is total, same as "never drew".
    pub(in crate::state) fn hud_tick(&mut self, now: Instant) {
        if !self.hud_enabled {
            if self.hud_hide_at.take().is_some() {
                self.os.hide_hud();
            }
            return;
        }
        let Some(deadline) = self.hud_hide_at else {
            return;
        };
        let armed = self
            .prefix
            .lock()
            .map(|prefix| prefix.is_armed(now))
            .unwrap_or(false);
        if armed {
            self.hud_hide_at = Some(now + self.hud_duration());
            return;
        }
        if now < deadline {
            return;
        }
        self.hud_hide_at = None;
        self.os.hide_hud();
    }

    fn hud_duration(&self) -> Duration {
        Duration::from_millis(self.hud_duration_ms.max(1))
    }
}

#[cfg(test)]
mod tests {
    fn card(number: usize, prefix_armed: bool) -> crate::adapter::HudCard {
        crate::adapter::HudCard {
            display_id: 1,
            number,
            name: "Browsing".into(),
            focused: false,
            prefix_armed,
        }
    }

    #[test]
    fn label_names_position_and_workspace() {
        assert_eq!(card(2, false).label(), "2 · Browsing");
    }

    #[test]
    fn prefix_armed_prepends_the_marker() {
        assert_eq!(card(2, true).label(), "» 2 · Browsing");
    }

    #[test]
    fn unnamed_display_reads_as_dash() {
        assert_eq!(card(0, false).label(), "—");
        assert_eq!(card(0, true).label(), "—");
    }
}
