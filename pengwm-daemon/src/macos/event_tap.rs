use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use core_foundation::mach_port::{
    CFMachPortCreateRunLoopSource, CFMachPortInvalidate, CFMachPortRef,
};
use core_foundation::runloop::{
    kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRef,
    CFRunLoopRemoveSource, CFRunLoopSourceRef,
};
use pengwm_core::command::Command;
use tokio::sync::mpsc;

use crate::config::keybinds::{find_keybind, KeybindConfig, ModifierFlags, MODIFIER_NONE};
use crate::event_loop::DaemonEvent;
use crate::macos::cf_ref::CFRef;
use crate::prefix::{PrefixKey, PrefixOutcome};

type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type CGEventMask = u64;

#[allow(non_upper_case_globals)]
const kCGEventKeyDown: u32 = 10;
#[allow(non_upper_case_globals)]
const kCGSessionEventTap: u32 = 1;
#[allow(non_upper_case_globals)]
const kCGHeadInsertEventTap: u32 = 0;
#[allow(non_upper_case_globals)]
const kCGEventTapOptionDefault: u32 = 0;
#[allow(non_upper_case_globals)]
const kCGEventTapDisabledByTimeout: u32 = 0xFFFF_FFFE;
#[allow(non_upper_case_globals)]
const kCGEventTapDisabledByUserInput: u32 = 0xFFFF_FFFF;

#[allow(non_upper_case_globals)]
const kCGKeyboardEventKeycode: u32 = 9;
#[allow(non_upper_case_globals)]
const kCGKeyboardEventAutorepeat: u32 = 8;

/// System shortcuts that must never be intercepted, even if user binds them.
const SYSTEM_SAFE_SHORTCUTS: &[(u16, ModifierFlags)] = &[
    (0x30, MODIFIER_CMD),                  // Cmd+Tab — app switcher
    (0x30, MODIFIER_CMD | MODIFIER_SHIFT), // Cmd+Shift+Tab — reverse app switcher
    (0x32, MODIFIER_CMD),                  // Cmd+` — same-app window cycling (backtick)
    (0x32, MODIFIER_CMD | MODIFIER_SHIFT), // Cmd+Shift+` — reverse window cycling
    (0x31, MODIFIER_CMD),                  // Cmd+Space — Spotlight
    (0x31, MODIFIER_CMD | MODIFIER_ALT),   // Cmd+Alt+Space — Finder Spotlight
];

/// Keeps the callback context alive until the source is detached. The raw
/// pointers keep this guard on the thread that owns the run loop.
pub struct EventTap {
    tap: CFRef<CFMachPortRef>,
    source: CFRef<CFRunLoopSourceRef>,
    run_loop: CFRunLoopRef,
    ctx: *mut Context,
}

impl Drop for EventTap {
    fn drop(&mut self) {
        // The callback runs only on this thread via this source. Detach and
        // invalidate it before reclaiming the box passed to Core Graphics.
        unsafe {
            CFRunLoopRemoveSource(self.run_loop, self.source.ptr(), kCFRunLoopDefaultMode);
            CFMachPortInvalidate(self.tap.ptr());
            drop(Box::from_raw(self.ctx));
        }
    }
}

pub fn start(
    event_tx: mpsc::Sender<DaemonEvent>,
    keybinds: Arc<Mutex<KeybindConfig>>,
    prefix: Arc<Mutex<PrefixKey>>,
) -> Option<EventTap> {
    let ctx = Box::into_raw(Box::new(Context {
        event_tx,
        keybinds: Arc::clone(&keybinds),
        prefix: Arc::clone(&prefix),
        tap: std::ptr::null_mut(),
    }));

    let event_mask = CGEventMaskBit(kCGEventKeyDown);

    unsafe {
        let Some(tap) = CFRef::from_create(CGEventTapCreate(
            kCGSessionEventTap,
            kCGHeadInsertEventTap,
            kCGEventTapOptionDefault, // active filter: returning NULL swallows the event
            event_mask,
            Some(event_tap_callback),
            ctx.cast(),
        )) else {
            drop(Box::from_raw(ctx));
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

            return None;
        };

        let Some(run_loop_source) = CFRef::from_create(CFMachPortCreateRunLoopSource(
            std::ptr::null(),
            tap.ptr(),
            0,
        )) else {
            CFMachPortInvalidate(tap.ptr());
            drop(Box::from_raw(ctx));
            log::error!("Failed to create CFRunLoopSource from event tap");
            return None;
        };

        (*ctx).tap = tap.ptr();
        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, run_loop_source.ptr(), kCFRunLoopDefaultMode);

        let guard = EventTap {
            tap,
            source: run_loop_source,
            run_loop,
            ctx,
        };
        eprintln!("✓ Global keybind tap active (Alt+h/j/k/l, arrows, etc.)");
        log::info!("CGEventTap started successfully");
        Some(guard)
    }
}

struct Context {
    event_tx: mpsc::Sender<DaemonEvent>,
    keybinds: Arc<Mutex<KeybindConfig>>,
    prefix: Arc<Mutex<PrefixKey>>,
    tap: CFMachPortRef,
}

unsafe extern "C" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    etype: u32,
    event: CGEventRef,
    refcon: *mut c_void,
) -> CGEventRef {
    let ctx = &*refcon.cast::<Context>();
    // Active taps are auto-disabled by the OS on timeout or user input changes.
    // Re-enable fail-open so keybinds keep working.
    if etype == kCGEventTapDisabledByTimeout || etype == kCGEventTapDisabledByUserInput {
        CGEventTapEnable(ctx.tap, true);
        return event;
    }

    let keycode = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode) as u16;
    let flags = CGEventGetFlags(event);

    let filtered_flags = flags & (MODIFIER_CMD | MODIFIER_ALT | MODIFIER_CTRL | MODIFIER_SHIFT);

    // Tap audit: one line per FILTERED keydown. When the tap is alive,
    // every chord with modifiers lands here; zero lines for chords the
    // physical keys pressed means the tap or the OS never delivered.
    log::debug!(
        "[tap] keydown keycode={:#04x} flags={:#x} filtered={:#x}",
        keycode,
        flags,
        filtered_flags
    );

    // The prefix owns unmodified keys while armed: a bare follow-up inherits
    // the prefix modifiers (`prefix, h` behaves like `alt-h`). Everything
    // else unmodified passes through untouched as before. One lock covers
    // both the armed check and the keydown handling — a per-keystroke
    // double lock was pure overhead on the input path.
    let mut prefix = match ctx.prefix.lock() {
        Ok(guard) => guard,
        Err(_) => return event, // fail-open: never block input on lock failure
    };
    let prefix_armed = prefix.is_armed(Instant::now());
    if filtered_flags == MODIFIER_NONE && !prefix_armed {
        return event;
    }

    // Never intercept system-critical shortcuts.
    if SYSTEM_SAFE_SHORTCUTS.contains(&(keycode, filtered_flags)) {
        return event;
    }

    let keybinds = match ctx.keybinds.lock() {
        Ok(guard) => guard,
        Err(_) => return event, // fail-open: never block input on lock failure
    };
    match prefix.on_keydown(keycode, filtered_flags, Instant::now(), &|kc, md| {
        find_keybind(kc, md, &keybinds)
    }) {
        PrefixOutcome::Armed => {
            return std::ptr::null_mut();
        }
        PrefixOutcome::Fire(command) => {
            log::debug!(
                "Prefix follow-up matched: keycode={}, command={:?}",
                keycode,
                command
            );
            let repeat = CGEventGetIntegerValueField(event, kCGKeyboardEventAutorepeat) != 0;
            if consume_command(&ctx.event_tx, command, repeat) {
                return std::ptr::null_mut();
            }
            return event;
        }
        PrefixOutcome::Passthrough => {}
    }
    if let Some(command) = find_keybind(keycode, filtered_flags, &keybinds) {
        log::debug!(
            "Keybind matched: keycode={}, command={:?}",
            keycode,
            command
        );
        let repeat = CGEventGetIntegerValueField(event, kCGKeyboardEventAutorepeat) != 0;
        if consume_command(&ctx.event_tx, command, repeat) {
            return std::ptr::null_mut(); // swallow: app never sees this keydown
        }
        return event;
    }

    event
}

fn consume_command(event_tx: &mpsc::Sender<DaemonEvent>, command: Command, repeat: bool) -> bool {
    // A held pop-out chord must not bounce between membership states, or
    // type its repeated keystrokes into the focused app.
    if repeat && command == Command::TogglePopout {
        return true;
    }
    // Otherwise fail open if the daemon can't receive the command.
    event_tx
        .try_send(DaemonEvent::Command(command, None))
        .is_ok()
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
            unsafe extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef,
        >,
        refcon: *mut c_void,
    ) -> CFMachPortRef;

    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetFlags(event: CGEventRef) -> CGEventFlags;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

type CGEventFlags = u64;

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    }

    #[test]
    #[ignore = "requires Accessibility permission and a GUI session"]
    fn native_tap_recovers_and_releases_callback_context() {
        assert!(crate::macos::ax_element::is_process_trusted());
        let (tx, rx) = mpsc::channel(1);
        let tap = start(
            tx,
            Arc::new(Mutex::new(KeybindConfig::new())),
            Arc::new(Mutex::new(PrefixKey::new(
                crate::prefix::PrefixConfig::default(),
            ))),
        )
        .expect("create native event tap");
        let port = tap.tap.clone();
        unsafe {
            CGEventTapEnable(port.ptr(), false);
            assert!(!CGEventTapIsEnabled(port.ptr()));
            // No proxy or event is available on this synthetic notification.
            event_tap_callback(
                std::ptr::null_mut(),
                kCGEventTapDisabledByTimeout,
                std::ptr::null_mut(),
                tap.ctx.cast(),
            );
            assert!(CGEventTapIsEnabled(port.ptr()));
        }
        drop(tap);
        assert_eq!(
            unsafe { core_foundation::mach_port::CFMachPortIsValid(port.ptr()) },
            0
        );
        assert!(rx.is_closed(), "the tap context's sender was dropped");
    }

    #[test]
    fn consume_popout_repeats_without_dispatching_them() {
        let (tx, mut rx) = mpsc::channel(4);
        assert!(consume_command(&tx, Command::TogglePopout, false));
        assert!(consume_command(&tx, Command::TogglePopout, true));
        assert!(matches!(
            rx.try_recv(),
            Ok(DaemonEvent::Command(Command::TogglePopout, None))
        ));
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
}
