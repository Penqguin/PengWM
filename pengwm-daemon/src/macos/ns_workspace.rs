//! NSWorkspace application lifecycle notifications.
//!
//! Subscribes to three Cocoa notifications:
//!   - NSWorkspaceDidLaunchApplicationNotification
//!   - NSWorkspaceDidActivateApplicationNotification
//!   - NSWorkspaceDidTerminateApplicationNotification
//!
//! These are received via the distributed notification center and forwarded
//! into the event loop as DaemonEvent::AppLaunched / AppActivated / AppTerminated.

use tokio::sync::mpsc;
use crate::event_loop::DaemonEvent;

/// Register NSWorkspace notification observers.
///
/// Must be called from the main thread (Cocoa requirement). The callbacks
/// fire on the main run loop, extract the PID from the notification's userInfo,
/// and send into the mpsc channel.
///
/// # Arguments
/// * `event_tx` — clone of the event loop sender.
pub fn observe(event_tx: mpsc::Sender<DaemonEvent>) {
    //  let center = NSWorkspace.sharedWorkspace.notificationCenter
    //
    //  center.addObserver(forName: NSWorkspaceDidLaunchApplicationNotification)
    //    callback: extract PID from notification.userInfo[NSApplicationProcessIdentifier]
    //              send DaemonEvent::AppLaunched(pid) into event_tx
    //
    //  center.addObserver(forName: NSWorkspaceDidActivateApplicationNotification)
    //    callback: extract PID, send DaemonEvent::AppActivated(pid)
    //
    //  center.addObserver(forName: NSWorkspaceDidTerminateApplicationNotification)
    //    callback: extract PID, send DaemonEvent::AppTerminated(pid)
    //
    //  Note: Keep the observer tokens alive for the lifetime of the daemon.
    todo!("subscribe to NSWorkspace notifications")
}
