use crate::core::bsp::BspTree;
use crate::core::types::SystemEvent;
use crate::core::geometry::Rect;
use crate::platform::WindowManagerBackend;
use crate::config::Config;
use tokio::sync::mpsc::Receiver;
use std::sync::Arc;

pub struct WindowManager {
    tree: BspTree,
    backend: Arc<dyn WindowManagerBackend + Send + Sync>,
    config: Config,
}

impl WindowManager {
    pub fn new(backend: Arc<dyn WindowManagerBackend + Send + Sync>, config: Config) -> Self {
        Self {
            tree: BspTree::new(),
            backend,
            config,
        }
    }

    pub async fn run(&mut self, mut event_rx: Receiver<SystemEvent>) {
        log::info!("WindowManager loop started.");
        
        // Initial monitor rect (stub - should be queried from backend)
        let monitor_rect = Rect::new(
            euclid::default::Point2D::new(0, 0),
            euclid::default::Size2D::new(1920, 1080)
        );

        while let Some(event) = event_rx.recv().await {
            match event {
                SystemEvent::WindowCreated(win) => {
                    log::info!("Handling WindowCreated: {:?}", win);
                    if self.backend.is_manageable(win) {
                        self.tree.insert_window(win, None, self.config.max_tiles);
                        self.apply_layout(monitor_rect).await;
                    }
                }
                SystemEvent::WindowDestroyed(win) => {
                    log::info!("Handling WindowDestroyed: {:?}", win);
                    // TODO: Implement window removal from BspTree
                }
                SystemEvent::WindowFocused(win) => {
                    log::info!("Handling WindowFocused: {:?}", win);
                }
                _ => {}
            }
        }
    }

    async fn apply_layout(&self, monitor_rect: Rect) {
        let layouts = self.tree.calculate_layout(
            monitor_rect,
            self.config.gap_inner,
            self.config.gap_outer
        );

        for (win, rect) in layouts {
            if let Err(e) = self.backend.set_window_rect(win, rect) {
                log::error!("Failed to set window rect for {:?}: {}", win, e);
            }
        }
    }
}
