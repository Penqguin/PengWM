use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use core_foundation::base::CFRelease;
use core_foundation::runloop::{
    kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopSourceRef,
};
use tokio::sync::mpsc;

use crate::config::keybinds::{find_keybind, KeybindConfig, ModifierFlags, MODIFIER_NONE};
use crate::event_loop::DaemonEvent;

type CGEventRef = *mut c_void;
type CGEventMask = u64;
type CFIndex = isize;

#[allow(non_upper_case_globals)]
const kCGEventKeyDown: u32 = 10;
#[allow(non_upper_case_globals)]
const kCGSessionEventTap: u32 = 1;
#[allow(non_upper_case_globals)]
const kCGHeadInsertEventTap: u32 = 0;

#[allow(non_upper_case_globals)]
const kCGKeyboardEventKeycode: u32 = 9;

/// System shortcuts that must never be intercepted, even if user binds them.
const SYSTEM_SAFE_SHORTCUTS: &[(u16, ModifierFlags)] = &[
    (0x30, MODIFIER_CMD),                  // Cmd+Tab — app switcher
    (0x30, MODIFIER_CMD | MODIFIER_SHIFT), // Cmd+Shift+Tab — reverse app switcher
    (0x32, MODIFIER_CMD),                  // Cmd+` — same-app window cycling (backtick)
    (0x32, MODIFIER_CMD | MODIFIER_SHIFT), // Cmd+Shift+` — reverse window cycling
    (0x31, MODIFIER_CMD),                  // Cmd+Space — Spotlight
    (0x31, MODIFIER_CMD | MODIFIER_ALT),   // Cmd+Alt+Space — Finder Spotlight
];

pub fn start(event_tx: mpsc::Sender<DaemonEvent>, keybinds: Arc<Mutex<KeybindConfig>>) {
    let ctx = Box::into_raw(Box::new(Context {
        event_tx,
        keybinds: Arc::clone(&keybinds),
    })) as *mut c_void;

    let event_mask = CGEventMaskBit(kCGEventKeyDown);

    unsafe {
        let tap = CGEventTapCreate(
            kCGSessionEventTap,
            kCGHeadInsertEventTap,
            1, // kCGEventTapOptionListenOnly = 1
            event_mask,
            Some(event_tap_callback),
            ctx,
        );

        if tap.is_null() {
            log::error!("Failed to create CGEventTap — check Accessibility permissions.");
            eprintln!("✗ Failed to create global keybind tap (CGEventTap).");
            eprintln!(
                "  Keybind interception won't work. CLI commands via `pengwm` will still work."
            );
            eprintln!();
            eprintln!("  To fix this, add pengwm to:");
            eprintln!("    System Settings → Privacy & Security → Accessibility");
            eprintln!();

            // Show the system permission prompt so the user can add the daemon.
            super::ax_element::request_trusted_access();

            let _ = Box::from_raw(ctx as *mut Context);
            return;
        }

        let run_loop_source: CFRunLoopSourceRef =
            CFMachPortCreateRunLoopSource(std::ptr::null_mut(), tap, 0) as CFRunLoopSourceRef;
        if run_loop_source.is_null() {
            log::error!("Failed to create CFRunLoopSource from event tap");
            CFRelease(tap);
            let _ = Box::from_raw(ctx as *mut Context);
            return;
        }

        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, run_loop_source, kCFRunLoopDefaultMode);

        // The run loop source is retained by the run loop; do not release it.

        eprintln!("✓ Global keybind tap active (Alt+h/j/k/l, arrows, etc.)");
        log::info!("CGEventTap started successfully");
    }
}

struct Context {
    event_tx: mpsc::Sender<DaemonEvent>,
    keybinds: Arc<Mutex<KeybindConfig>>,
}

unsafe extern "C" fn event_tap_callback(
    _proxy: CGEventRef,
    _type: u32,
    event: CGEventRef,
    refcon: *mut c_void,
) -> CGEventRef {
    let ctx = &*(refcon as *const Context);

    let keycode = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode) as u16;
    let flags = CGEventGetFlags(event);

    let filtered_flags = flags & (MODIFIER_CMD | MODIFIER_ALT | MODIFIER_CTRL | MODIFIER_SHIFT);

    // Always pass through unmodified keypresses (Tab, Space, Enter, etc.)
    // so the active event tap cannot interfere with normal keyboard input.
    if filtered_flags == MODIFIER_NONE {
        return event;
    }

    // Never intercept system-critical shortcuts.
    if SYSTEM_SAFE_SHORTCUTS.contains(&(keycode, filtered_flags)) {
        return event;
    }

    let keybinds = ctx.keybinds.lock().expect("keybind mutex poisoned");
    if let Some(command) = find_keybind(keycode, filtered_flags, &keybinds) {
        log::debug!(
            "Keybind matched: keycode={}, command={:?}",
            keycode,
            command
        );
        let _ = ctx.event_tx.try_send(DaemonEvent::Keybind(command));
        return event;
    }

    event
}

const MODIFIER_CMD: ModifierFlags = 0x0010_0000;
const MODIFIER_ALT: ModifierFlags = 0x0008_0000;
const MODIFIER_CTRL: ModifierFlags = 0x0004_0000;
const MODIFIER_SHIFT: ModifierFlags = 0x0002_0000;

#[allow(non_snake_case)]
fn CGEventMaskBit(event_type: u32) -> CGEventMask {
    1 << event_type
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: CGEventMask,
        callback: Option<
            unsafe extern "C" fn(CGEventRef, u32, CGEventRef, *mut c_void) -> CGEventRef,
        >,
        refcon: *mut c_void,
    ) -> CGEventRef;

    fn CFMachPortCreateRunLoopSource(
        allocator: *mut c_void,
        port: CGEventRef,
        order: CFIndex,
    ) -> *mut c_void;

    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetFlags(event: CGEventRef) -> CGEventFlags;
}

type CGEventFlags = u64;
