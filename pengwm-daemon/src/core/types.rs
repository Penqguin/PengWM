use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WindowId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MonitorId(pub usize);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemEvent {
    WindowCreated(WindowId),
    WindowDestroyed(WindowId),
    WindowFocused(WindowId),
    MonitorAdded(MonitorId),
    MonitorRemoved(MonitorId),
    AppLaunched(i32), // PID of the new application
}
