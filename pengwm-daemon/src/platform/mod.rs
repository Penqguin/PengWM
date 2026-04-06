use async_trait::async_trait;
use crate::core::geometry::Rect;
use crate::core::types::{WindowId, SystemEvent};
use tokio::sync::mpsc::Sender;
use anyhow::Result;

#[async_trait]
pub trait WindowManagerBackend {
    /// Subscribe to system events (creation, destruction, focus)
    async fn subscribe(&self, event_sender: Sender<SystemEvent>);

    /// Move and resize a window to specific coordinates
    fn set_window_rect(&self, window: WindowId, rect: Rect) -> Result<()>;

    /// Check if a window should be managed (is it a normal app window?)
    fn is_manageable(&self, window: WindowId) -> bool;

    /// Get the current focused window
    fn get_focused_window(&self) -> Result<WindowId>;
}

pub mod windows;
pub mod macos;
