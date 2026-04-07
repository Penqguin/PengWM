//! Core window management logic.
//! Orchestrates the BSP tree, backend interactions, and UI synchronization.

use crate::core::bsp::BspTree;
use crate::core::types::SystemEvent;
use crate::core::geometry::Rect;
use crate::platform::WindowManagerBackend;
use crate::config::Config;
use crate::ipc::{IpcServer, UiState, WindowInfo};
use tokio::sync::mpsc::Receiver;
use std::sync::Arc;

/// The central coordinator for window management.
pub struct WindowManager {
    /// The Binary Space Partitioning tree that stores window positions.
    tree: BspTree,
    /// The platform-specific backend (macOS or Windows).
    backend: Arc<dyn WindowManagerBackend + Send + Sync>,
    /// Current configuration settings.
    config: Config,
    /// IPC server for broadcasting state updates to the UI.
    ipc_server: Arc<IpcServer>,
}

impl WindowManager {
    /// Creates a new WindowManager instance.
    pub fn new(
        backend: Arc<dyn WindowManagerBackend + Send + Sync>,
        config: Config,
        ipc_server: Arc<IpcServer>,
    ) -> Self {
        Self {
            tree: BspTree::new(),
            backend,
            config,
            ipc_server,
        }
    }

    /// The main execution loop of the Window Manager.
    /// Listens for system events and updates the layout accordingly.
    pub async fn run(&mut self, mut event_rx: Receiver<SystemEvent>) {
        log::info!("WindowManager loop started.");

        // Initial monitor geometry. 
        // TODO: In a multi-monitor setup, this should be queried per event.
        let monitor_rect = Rect::new(
            euclid::default::Point2D::new(0, 0),
            euclid::default::Size2D::new(1920, 1080)
        );

        // Continuous loop to process window events.
        while let Some(event) = event_rx.recv().await {
            match event {
                SystemEvent::WindowCreated(win) => {
                    log::info!("Handling WindowCreated: {:?}", win);
                    // Avoid managing the same window multiple times.
                    if self.tree.contains_window(win) {
                        continue;
                    }
                    // Only manage windows that pass the backend's filtering rules.
                    if self.backend.is_manageable(win) {
                        // Insert the window into the BSP tree.
                        self.tree.insert_window(win, None, self.config.max_tiles);
                        // Re-calculate and apply the layout to all windows.
                        self.apply_layout(monitor_rect).await;
                        // Notify the UI of the change.
                        self.sync_ui();
                    } else {
                        // If it's not manageable, release any retained reference.
                        self.backend.release_window(win);
                    }
                }
                SystemEvent::WindowDestroyed(win) => {
                    log::info!("Handling WindowDestroyed: {:?}", win);
                    // TODO: Implement window removal logic in the BspTree.
                    // Release our retained reference to the window element.
                    self.backend.release_window(win);
                    self.sync_ui();
                }
                SystemEvent::WindowFocused(win) => {
                    log::info!("Handling WindowFocused: {:?}", win);
                    // Focus changes might update UI elements like borders.
                    self.sync_ui();
                }
                _ => {}
            }
        }
    }

    /// Calculates the rectangles for all windows in the tree and applies them via the backend.
    async fn apply_layout(&self, monitor_rect: Rect) {
        let layouts = self.tree.calculate_layout(
            monitor_rect,
            self.config.gap_inner,
            self.config.gap_outer
        );

        for (win, rect) in layouts {
            // Tell the OS to move/resize the window.
            if let Err(e) = self.backend.set_window_rect(win, rect) {
                log::error!("Failed to set window rect for {:?}: {}", win, e);
            }
        }
    }

    /// Synchronizes the current window manager state with all connected UI clients.
    fn sync_ui(&self) {
        // Use a default monitor size for coordinate normalization in the UI.
        let monitor_rect = Rect::new(
            euclid::default::Point2D::new(0, 0),
            euclid::default::Size2D::new(1920, 1080)
        );

        // Calculate the current positions of all windows.
        let layouts = self.tree.calculate_layout(
            monitor_rect,
            self.config.gap_inner,
            self.config.gap_outer
        );

        // Convert the internal layout into a UI-friendly format.
        let windows = layouts.into_iter().map(|(id, rect)| {
            WindowInfo {
                id: id.0,
                title: format!("Window {}", id.0), // TODO: Fetch actual window title from backend.
                x: rect.min_x(),
                y: rect.min_y(),
                width: rect.width() as u32,
                height: rect.height() as u32,
            }
        }).collect();

        let state = UiState {
            windows,
            focused_window: None, // TODO: Track focus in WindowManager.
        };
        // Push the new state to all connected IPC clients.
        self.ipc_server.broadcast_state(state);
    }
}

