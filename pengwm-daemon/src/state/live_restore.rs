use pengwm_core::workspace::{PendingKind, PendingWindow};

use super::session::LivePlacement;
use super::store::{ParkedEntry, RoutingVerdict};
use super::StateManager;

impl StateManager {
    pub(super) fn capture_live_windows(&self) -> Vec<LivePlacement> {
        let mut records = Vec::new();
        for &id in self.store.all_window_pids().keys() {
            if self.store.routing_verdict(id) != Some(RoutingVerdict::Tiled) {
                continue;
            }
            let Some(identity) = self.store.identity(id) else {
                continue;
            };
            let parked = self.store.parked_entry(id);
            let Some(idx) = self
                .find_workspace_of_any(id)
                .or_else(|| parked.map(|p| p.idx))
            else {
                continue;
            };
            let Some(ws) = self.workspaces.get(idx) else {
                continue;
            };
            let pending = ws
                .pending_window(id)
                .or_else(|| parked.and_then(|p| p.pending));
            records.push(LivePlacement {
                identity: identity.clone(),
                workspace: ws.name.clone(),
                order: self.store.opening_order(id).unwrap_or(0),
                pending: pending.map(|p| p.kind),
                parked: parked.map(|p| p.reason),
            });
        }
        records.sort_by_key(|p| p.order);
        records
    }

    pub(super) fn restore_live_windows(&mut self, records: &[LivePlacement]) {
        let mut records: Vec<_> = records.iter().collect();
        // Rebuild existing tiles before offering their slots to waiting windows.
        records.sort_by_key(|p| (p.pending.is_some() || p.parked.is_some(), p.order));
        for record in &records {
            let id = record.identity.window_id;
            if self.find_workspace_of_any(id).is_some() || self.store.is_parked(id) {
                continue;
            }
            if self.store.pid_for(id) != Some(record.identity.pid) {
                continue;
            }
            let Some(idx) = self
                .workspaces
                .iter()
                .position(|ws| ws.name == record.workspace)
            else {
                continue;
            };
            let duplicates = records
                .iter()
                .filter(|other| {
                    other.identity.pid == record.identity.pid
                        && other.identity.accessibility_id == record.identity.accessibility_id
                })
                .count();
            if duplicates != 1 || self.os.window_identity(id).as_ref() != Some(&record.identity) {
                continue;
            }
            let class = self
                .os
                .window_class_fresh(id)
                .or_else(|| self.os.window_kind(id));
            let app = self.app_identity(record.identity.pid);
            if !self.window_is_tileable(class, &app) {
                continue;
            }
            self.store.note_routed(id, RoutingVerdict::Tiled);
            self.store.note_identity(id, Some(record.identity.clone()));
            self.store.restore_order(id, record.order);
            let window = PendingWindow {
                window_id: id,
                kind: record.pending.unwrap_or(PendingKind::Returning),
                order: record.order,
            };
            let hidden = self
                .os
                .windows_are_hidden(&[id])
                .get(&id)
                .copied()
                .unwrap_or(false);
            let fullscreen = self.os.window_is_fullscreen(id) && self.fullscreen_parking_enabled();
            if hidden || fullscreen {
                self.store.restore_park(
                    id,
                    ParkedEntry {
                        idx,
                        popup: false,
                        pending: record.pending.map(|_| window),
                        reason: if hidden {
                            super::store::ParkReason::Hide
                        } else {
                            super::store::ParkReason::Fullscreen
                        },
                    },
                );
            } else {
                self.workspaces[idx].add_pending(window);
                if record.pending.is_none() {
                    self.fill_workspace_slots(idx);
                }
            }
        }
        for idx in 0..self.workspaces.len() {
            self.fill_workspace_slots(idx);
            if self.displays.is_visible(idx, &self.workspaces) {
                self.apply_layout(idx);
            }
        }
        self.session_dirty = true;
    }
}
