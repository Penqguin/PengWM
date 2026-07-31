use std::ffi::c_void;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSString};
use tokio::sync::mpsc;

use crate::event_loop::DaemonEvent;

#[allow(non_upper_case_globals)]
static NSApplicationProcessIdentifier: &str = "NSApplicationProcessIdentifier";

pub fn observe(event_tx: mpsc::Sender<DaemonEvent>) {
    // NSWorkspace notifications are posted to the workspace's own
    // notification center, NOT the default notification center.
    let ws = NSWorkspace::sharedWorkspace();
    let center = ws.notificationCenter();

    let ctx = Box::into_raw(Box::new(event_tx)) as *mut c_void;

    unsafe {
        add_observer(&center, ctx, NSWorkspaceDidLaunchApplicationNotification, 0);
        add_observer(
            &center,
            ctx,
            NSWorkspaceDidActivateApplicationNotification,
            1,
        );
        add_observer(
            &center,
            ctx,
            NSWorkspaceDidTerminateApplicationNotification,
            2,
        );
    }
}

use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidActivateApplicationNotification,
    NSWorkspaceDidLaunchApplicationNotification, NSWorkspaceDidTerminateApplicationNotification,
};

pub fn running_app_pids() -> Vec<i32> {
    let ws = NSWorkspace::sharedWorkspace();
    ws.runningApplications()
        .into_iter()
        .filter(|app| {
            app.activationPolicy() == objc2_app_kit::NSApplicationActivationPolicy::Regular
        })
        .map(|app| app.processIdentifier())
        .collect()
}

/// Resolve a process ID to its bundle identifier (e.g. "com.spotify.client").
pub fn bundle_id_for_pid(pid: i32) -> Option<String> {
    let ws = NSWorkspace::sharedWorkspace();
    for app in ws.runningApplications() {
        if app.processIdentifier() == pid {
            return app.bundleIdentifier().map(|s| s.to_string());
        }
    }
    None
}

fn add_observer(center: &NSNotificationCenter, ctx: *mut c_void, name: &NSString, event_type: u8) {
    let block = RcBlock::new(move |notification: NonNull<NSNotification>| {
        let notif = unsafe { notification.as_ref() };
        if let Some(pid) = extract_pid(notif) {
            let event = match event_type {
                0 => DaemonEvent::AppLaunched(pid),
                1 => DaemonEvent::AppActivated(pid),
                _ => DaemonEvent::AppTerminated(pid),
            };
            log::debug!("NSWorkspace notification: pid={}", pid);
            let tx = unsafe { &*(ctx as *const mpsc::Sender<DaemonEvent>) };
            let _ = tx.try_send(event);
        }
    });

    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
    };
    std::mem::forget(token);
}

fn extract_pid(notification: &NSNotification) -> Option<i32> {
    let user_info = notification.userInfo()?;
    let key = NSString::from_str(NSApplicationProcessIdentifier);
    unsafe {
        let value: Option<Retained<NSObject>> = msg_send![&user_info, objectForKey: &*key];
        value.map(|obj| {
            let pid: i32 = msg_send![&obj, intValue];
            pid
        })
    }
}

use objc2::msg_send;
use objc2::runtime::{AnyClass, NSObject};

/// Returns the height of the menu bar on the primary display, in points.
/// Returns 0 if the menu bar is auto-hidden.
pub fn menu_bar_height() -> f64 {
    let cls = match AnyClass::get(c"NSScreen") {
        Some(c) => c,
        None => return 0.0,
    };
    unsafe {
        let screens: *mut NSObject = msg_send![cls, screens];
        if screens.is_null() {
            return 0.0;
        }
        let count: usize = msg_send![screens, count];
        if count == 0 {
            return 0.0;
        }
        let first: *mut NSObject = msg_send![screens, objectAtIndex: 0usize];
        if first.is_null() {
            return 0.0;
        }
        let frame: objc2_foundation::NSRect = msg_send![first, frame];
        let visible: objc2_foundation::NSRect = msg_send![first, visibleFrame];

        let bottom_inset = visible.origin.y;
        let total_inset = frame.size.height - visible.size.height;
        let top_inset = total_inset - bottom_inset;
        top_inset.max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_identifier_constant() {
        assert_eq!(
            NSApplicationProcessIdentifier,
            "NSApplicationProcessIdentifier"
        );
    }

    #[test]
    fn observe_function_signature() {
        fn _type_check(_f: fn(mpsc::Sender<DaemonEvent>)) {}
        _type_check(observe);
    }

    #[test]
    fn event_ctors() {
        assert!(matches!(
            DaemonEvent::AppLaunched(42),
            DaemonEvent::AppLaunched(42)
        ));
        assert!(matches!(
            DaemonEvent::AppActivated(42),
            DaemonEvent::AppActivated(42)
        ));
        assert!(matches!(
            DaemonEvent::AppTerminated(42),
            DaemonEvent::AppTerminated(42)
        ));
    }
}
