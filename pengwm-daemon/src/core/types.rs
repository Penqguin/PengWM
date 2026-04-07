//! Common types used throughout the window manager core.

use serde::{Serialize, Deserialize};

/// A unique identifier for a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WindowId(pub usize);

/// A unique identifier for a monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MonitorId(pub usize);

/// Events emitted by the underlying platform's windowing system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemEvent {
    /// A new window has been created.
    WindowCreated(WindowId),
    /// A window has been destroyed.
    WindowDestroyed(WindowId),
    /// A window has gained focus.
    WindowFocused(WindowId),
    /// A new monitor has been added.
    MonitorAdded(MonitorId),
    /// A monitor has been removed.
    MonitorRemoved(MonitorId),
    /// An application has been launched.
    AppLaunched(i32), // PID of the new application
}
