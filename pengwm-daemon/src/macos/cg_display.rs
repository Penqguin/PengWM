use std::ffi::c_void;

use core_graphics::display::{CGDirectDisplayID, CGDisplay};
use tokio::sync::mpsc;

use crate::adapter::DisplayInfo;
use crate::event_loop::DaemonEvent;

pub fn active_displays() -> Vec<DisplayInfo> {
    let ids = match CGDisplay::active_displays() {
        Ok(ids) => ids,
        Err(_) => return Vec::new(),
    };

    ids.iter()
        .map(|&id| {
            let display = CGDisplay::new(id);
            let bounds = display.bounds();
            DisplayInfo {
                id,
                origin: (bounds.origin.x as i32, bounds.origin.y as i32),
                size: (bounds.size.width as u32, bounds.size.height as u32),
            }
        })
        .collect()
}

pub fn primary_display_id() -> u32 {
    unsafe { core_graphics::display::CGMainDisplayID() }
}

type CGDisplayReconfigurationCallBack = unsafe extern "C" fn(
    display: CGDirectDisplayID,
    flags: CGDisplayChangeFlags,
    refcon: *mut c_void,
);

type CGDisplayChangeFlags = u32;

#[allow(non_upper_case_globals)]
const kCGDisplayAddFlag: CGDisplayChangeFlags = 1 << 0;
#[allow(non_upper_case_globals)]
const kCGDisplayRemoveFlag: CGDisplayChangeFlags = 1 << 1;
#[allow(non_upper_case_globals)]
const kCGDisplaySetModeFlag: CGDisplayChangeFlags = 1 << 3;

use std::sync::atomic::AtomicPtr;

static REFCON: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub fn register_hotplug_callback(event_tx: mpsc::Sender<DaemonEvent>) {
    let ctx = Box::into_raw(Box::new(event_tx)) as *mut c_void;
    REFCON.store(ctx, std::sync::atomic::Ordering::Relaxed);

    unsafe {
        CGDisplayRegisterReconfigurationCallback(Some(hotplug_callback), ctx);
    }
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGDisplayRegisterReconfigurationCallback(
        callback: Option<CGDisplayReconfigurationCallBack>,
        refcon: *mut c_void,
    );
    fn CGDisplayRemoveReconfigurationCallback(
        callback: Option<CGDisplayReconfigurationCallBack>,
        refcon: *mut c_void,
    );
}

unsafe extern "C" fn hotplug_callback(
    display: CGDirectDisplayID,
    flags: CGDisplayChangeFlags,
    refcon: *mut c_void,
) {
    let tx = &*(refcon as *const mpsc::Sender<DaemonEvent>);

    if flags & kCGDisplayAddFlag != 0 {
        log::info!("Display added: {}", display);
        let _ = tx.try_send(DaemonEvent::MonitorAdded(display));
    }
    if flags & kCGDisplayRemoveFlag != 0 {
        log::info!("Display removed: {}", display);
        let _ = tx.try_send(DaemonEvent::MonitorRemoved(display));
    }
    if flags & kCGDisplaySetModeFlag != 0 {
        log::info!("Display resized: {}", display);
        let _ = tx.try_send(DaemonEvent::MonitorResized(display));
    }
}

pub fn unregister_hotplug_callback() {
    let ctx = REFCON.swap(std::ptr::null_mut(), std::sync::atomic::Ordering::Relaxed);
    unsafe {
        CGDisplayRemoveReconfigurationCallback(Some(hotplug_callback), ctx);
    }
    if !ctx.is_null() {
        unsafe {
            let _ = Box::from_raw(ctx as *mut mpsc::Sender<DaemonEvent>);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_info_struct_size() {
        assert_eq!(std::mem::size_of::<DisplayInfo>(), 20); // u32 + (i32,i32) + (u32,u32)
    }

    #[test]
    fn display_flag_constants() {
        assert_eq!(kCGDisplayAddFlag, 1);
        assert_eq!(kCGDisplayRemoveFlag, 2);
        assert_eq!(kCGDisplaySetModeFlag, 8);
    }

    #[test]
    fn primary_display_id_function_exists() {
        fn _type_check(_f: fn() -> u32) {}
        _type_check(primary_display_id);
    }

    #[test]
    fn active_displays_function_exists() {
        fn _type_check(_f: fn() -> Vec<DisplayInfo>) {}
        _type_check(active_displays);
    }
}
