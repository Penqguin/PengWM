//! Display-geometry probe (diagnostic tool, read-only). Prints, for every
//! live display: the daemon's own `active_displays()` view (inset-clamped
//! AX space), the raw CG bounds, the AppKit `NSScreen` frames, the
//! per-display top insets the WM folds into tiling, and any learned top
//! bands. Run from the repo root: `cargo run -p pengwm-daemon --example display_probe`

use core_graphics::display::{CGDisplay};

fn main() {
    let ids = match CGDisplay::active_displays() {
        Ok(ids) => ids,
        Err(e) => {
            println!("CGDisplay::active_displays() failed: {e:?}");
            return;
        }
    };
    println!("CGDisplay::active_displays() ids: {ids:?}");
    let mut raw_map = Vec::new();
    for &id in &ids {
        let b = CGDisplay::new(id).bounds();
        raw_map.push((id, (b.origin.x, b.origin.y), (b.size.width, b.size.height)));
        println!(
            "raw CG bounds id={}: origin=({:?}) size=({:?})",
            id, (b.origin.x, b.origin.y), (b.size.width, b.size.height)
        );
    }

    println!("---- daemon view (inset-clamped AX space) ----");
    for info in pengwm_daemon::macos::cg_display::active_displays() {
        println!(
            "active_display id={}: origin={:?} size={:?}",
            info.id, info.origin, info.size
        );
    }

    println!("---- NSWindow side: NSScreen frames + NSScreenNumber ----");
    crate_ns_screens();

    println!("---- AppKit visibleFrame top insets (the menu-bar math) ----");
    for (id, inset) in pengwm_daemon::macos::ns_workspace::screen_top_insets() {
        println!("top inset id={}: {}px", id, inset);
    }

    println!("---- learned top bands (renormalization memory) ----");
    for (id, band) in pengwm_daemon::macos::cg_display::learned_top_bands() {
        println!("learned band id={}: {}px", id, band);
    }

    println!("---- primary ----");
    println!("CGMainDisplayID: {:?}", unsafe { core_graphics::display::CGMainDisplayID() });
}

fn crate_ns_screens() {
    use objc2::runtime::{AnyClass, NSObject, AnyObject};
    use objc2::{msg_send};
    objc2::rc::autoreleasepool(|_| {
        let cls = match AnyClass::get(c"NSScreen") {
            Some(c) => c,
            None => {
                println!("NSScreen class not found?!");
                return;
            }
        };
        unsafe {
            let screens: *mut NSObject = msg_send![cls, screens];
            if screens.is_null() {
                println!("NSScreen.screens returned nil — AppKit not initialized?");
                return;
            }
            let keys: usize = 0;
            let _ = keys;
            let count: usize = msg_send![&*screens, count];
            for i in 0..count {
                let screen: *mut AnyObject = msg_send![&*screens, objectAtIndex: i];
                if screen.is_null() {
                    continue;
                }
                let desc: *mut AnyObject = msg_send![&*screen, deviceDescription];
                if desc.is_null() {
                    continue;
                }
                let key = objc2_foundation::NSString::from_str("NSScreenNumber");
                let value: Option<objc2::rc::Retained<objc2_foundation::NSNumber>> =
                    msg_send![desc, objectForKey: &*key];
                let Some(num) = value else {
                    println!("screen #{i}: no NSScreenNumber");
                    continue;
                };
                let display_id: u32 = msg_send![&num, unsignedIntValue];
                let frame: objc2_foundation::NSRect = msg_send![&*screen, frame];
                let visible: objc2_foundation::NSRect = msg_send![&*screen, visibleFrame];
                println!(
                    "screen #{i} (NSScreenNumber {}): frame origin=({:?}) size=({:?})  visibleFrame origin=({:?}) size=({:?})",
                    display_id,
                    (frame.origin.x, frame.origin.y),
                    (frame.size.width, frame.size.height),
                    (visible.origin.x, visible.origin.y),
                    (visible.size.width, visible.size.height)
                );
            }
        }
    });
}
