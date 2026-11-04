use std::ffi::c_void;
use std::ptr::NonNull;
use std::collections::HashMap;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSNumber, NSString};
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
        add_wake_observer(&center, ctx, NSWorkspaceDidWakeNotification);
        add_wake_observer(&center, ctx, NSWorkspaceScreensDidWakeNotification);
        add_space_observer(&center, ctx, NSWorkspaceActiveSpaceDidChangeNotification);
    }
}

use objc2_app_kit::{
    NSWorkspace, NSWorkspaceActiveSpaceDidChangeNotification,
    NSWorkspaceDidActivateApplicationNotification,
    NSWorkspaceDidLaunchApplicationNotification, NSWorkspaceDidTerminateApplicationNotification,
    NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidWakeNotification,
};

pub fn running_app_pids() -> Vec<i32> {
    objc2::rc::autoreleasepool(|_pool| {
        let ws = NSWorkspace::sharedWorkspace();
        ws.runningApplications()
            .into_iter()
            .filter(|app| {
                app.activationPolicy() == objc2_app_kit::NSApplicationActivationPolicy::Regular
            })
            .map(|app| app.processIdentifier())
            .collect()
    })
}

/// Every pid NSWorkspace reports as running, regardless of activation
/// policy. `running_app_pids` filters to Regular apps because routing
/// cares about real apps; liveness checks must not — accessory-policy apps
/// own windows too, and the dead-pid sweep reading their absence as death
/// would untrack them every tick.
pub fn running_pids_any_policy() -> Vec<i32> {
    objc2::rc::autoreleasepool(|_pool| {
        let ws = NSWorkspace::sharedWorkspace();
        ws.runningApplications()
            .into_iter()
            .map(|app| app.processIdentifier())
            .collect()
    })
}

/// Resolve a process ID to its bundle identifier (e.g. "com.spotify.client").
pub fn bundle_id_for_pid(pid: i32) -> Option<String> {
    objc2::rc::autoreleasepool(|_pool| {
        let ws = NSWorkspace::sharedWorkspace();
        for app in ws.runningApplications() {
            if app.processIdentifier() == pid {
                return app.bundleIdentifier().map(|s| s.to_string());
            }
        }
        None
    })
}

/// Resolve a process ID to its human-readable display name (e.g. "Spotify"),
/// as shown in the Dock. Falls back to the bundle id when the localized name
/// is unavailable.
pub fn localized_name_for_pid(pid: i32) -> Option<String> {
    objc2::rc::autoreleasepool(|_pool| {
        let ws = NSWorkspace::sharedWorkspace();
        for app in ws.runningApplications() {
            if app.processIdentifier() == pid {
                return Some(
                    app.localizedName()
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                )
                .filter(|s| !s.is_empty());
            }
        }
        None
    })
}

/// One scan of `runningApplications`, answering every pid's identity at
/// once: `(bundle id, display name)` per running pid. The shape the
/// identity memo resolves misses against — a sweep that needs identities
/// for A pids pays this single O(apps) scan instead of two scans per pid.
pub fn running_app_identities() -> HashMap<i32, (Option<String>, Option<String>)> {
    objc2::rc::autoreleasepool(|_pool| {
        let ws = NSWorkspace::sharedWorkspace();
        ws.runningApplications()
            .into_iter()
            .map(|app| {
                let pid = app.processIdentifier();
                let bundle = app.bundleIdentifier().map(|s| s.to_string());
                let name = app
                    .localizedName()
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty());
                (pid, (bundle, name))
            })
            .collect()
    })
}

fn add_observer(center: &NSNotificationCenter, ctx: *mut c_void, name: &NSString, event_type: u8) {
    let block = RcBlock::new(move |notification: NonNull<NSNotification>| {
        // One pool per notification: the block runs on the daemon's main
        // CFRunLoop thread, which has no NSApplication event loop to drain
        // autorelease pools — without this, AppKit/Foundation temporaries
        // from every notification accumulate as reachable memory forever.
        objc2::rc::autoreleasepool(|_pool| {
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
    });

    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
    };
    std::mem::forget(token);
}

/// Wake notifications carry no pid — they just mean "resync everything".
fn add_wake_observer(center: &NSNotificationCenter, ctx: *mut c_void, name: &NSString) {
    let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        log::info!("NSWorkspace wake notification — resyncing");
        let tx = unsafe { &*(ctx as *const mpsc::Sender<DaemonEvent>) };
        let _ = tx.try_send(DaemonEvent::SystemWoke);
    });

    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
    };
    std::mem::forget(token);
}

/// A native-Space switch carries no pid — it forces the Space gate's
/// detector refresh (ADR-0007) without waiting for the tick poll.
fn add_space_observer(center: &NSNotificationCenter, ctx: *mut c_void, name: &NSString) {
    let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        let tx = unsafe { &*(ctx as *const mpsc::Sender<DaemonEvent>) };
        let _ = tx.try_send(DaemonEvent::SpaceChanged);
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

/// Per-display menu-bar top inset, in points, keyed by CG display id
/// (`NSScreenNumber`, the id in each screen's device description). Reads
/// *every* screen's `visibleFrame` — macOS shows a menu bar on secondaries
/// too (incl. Sidecar/iPad panels and the "display has separate Spaces"
/// menu bar), and tiling that ignores it gets renormalized by the OS on
/// every write. Auto-hidden menu bars and screens with no bar report 0 and
/// are best dropped by the caller. Wrapped in a pool: the screens
/// enumeration autoreleases on the daemon's pool-less CFRunLoop thread.
#[cfg(target_os = "macos")]
pub fn screen_top_insets() -> Vec<(u32, f64)> {
    let mut out = Vec::new();
    objc2::rc::autoreleasepool(|_pool| {
        let cls = match AnyClass::get(c"NSScreen") {
            Some(c) => c,
            None => return,
        };
        unsafe {
            let screens: *mut NSObject = msg_send![cls, screens];
            if screens.is_null() {
                return;
            }
            let count: usize = msg_send![screens, count];
            for i in 0..count {
                let screen: *mut objc2::runtime::AnyObject =
                    msg_send![screens, objectAtIndex: i];
                if screen.is_null() {
                    continue;
                }
                let desc: *mut objc2::runtime::AnyObject =
                    msg_send![screen, deviceDescription];
                if desc.is_null() {
                    continue;
                }
                let key = NSString::from_str("NSScreenNumber");
                let value: Option<Retained<NSNumber>> =
                    msg_send![desc, objectForKey: &*key];
                let Some(num) = value else { continue };
                let display_id: u32 = msg_send![&num, unsignedIntValue];
                let frame: objc2_foundation::NSRect = msg_send![screen, frame];
                let visible: objc2_foundation::NSRect = msg_send![screen, visibleFrame];
                // Both rects are in *global* (bottom-left origin) display
                // space; only their difference is screen-local. Subtracting
                // the frame's own origin was skipped by the old primary-only
                // helper because the primary sits at y=0 — on any other
                // screen, `visible.origin.y` is its global position, not a
                // dock inset.
                let bottom_inset = visible.origin.y - frame.origin.y;
                let total_inset = frame.size.height - visible.size.height;
                let top_inset = total_inset - bottom_inset;
                out.push((display_id, top_inset.max(0.0)));
            }
        }
    });
    out
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
