//! Transient workspace pill renderer (Spec 0002).
//! Display identity is matched against live AppKit geometry on the main thread.
//!
//! Updates arrive via the connection thread into a shared `HudSlot`
//! (the enum lives in `main.rs` — the connection thread is not
//! macOS-gated); a repeating NSTimer on the main thread drains it — the
//! connection thread never touches AppKit, and the timer tick is the
//! only main-thread surface that MUST touch it (NSWindow rules).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFont, NSScreen, NSTextField, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    NSNumber, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};
use pengwm_core::command::PillCard;
use std::time::Instant;

use crate::HudSlotRef;

/// Fixed pill geometry (points). Cross-reference the daemon-side spec
/// screenshot expectations: monospaced, dark material, centered.
const PADDING_X: f64 = 14.0;
const PADDING_Y: f64 = 7.0;
const CORNER_RADIUS: f64 = 9.0;
const FONT_SIZE: f64 = 12.0;
/// Dead-center: the pill fullly covers a mid-screen strip momentarily —
/// that's the point ("where did things land"), and it's click-through.
const WINDOW_LEVEL: isize = 3;

struct HudPanel {
    window: Retained<HudWindow>,
    label: Retained<NSTextField>,
}

define_class!(
    /// An `NSWindow` that refuses AppKit's frame-sanity clamp. With real
    /// screen-list maintenance this should never trigger, but the cost
    /// is nil and the failure mode it guards (a clamp dragging the pill
    /// to the primary) was expensive to diagnose twice.
    #[unsafe(super = NSWindow)]
    #[thread_kind = MainThreadOnly]
    #[name = "PengwmHudWindow"]
    struct HudWindow;

    // SAFETY: NSObjectProtocol imposes no runtime requirements.
    unsafe impl NSObjectProtocol for HudWindow {}

    impl HudWindow {
        // SAFETY: matches `constrainFrameRect:toScreen:` — the hook
        // AppKit calls to validate a window frame against a screen.
        #[unsafe(method(constrainFrameRect:toScreen:))]
        unsafe fn constrain_frame_rect(&self, frame_rect: NSRect, _screen: Option<&NSScreen>) -> NSRect {
            frame_rect
        }
    }
);

/// One panel per display id, created lazily, restyled in place, ordered
/// out on hide — pill text size changes per flash, so every show is a
/// full re-fit (text, accent color, frame) of the surviving panels.
pub struct HudPanels {
    panels: HashMap<u32, HudPanel>,
}

impl HudPanels {
    pub fn new() -> Self {
        Self {
            panels: HashMap::new(),
        }
    }

    /// Draw every card (UI thread only — asserted by the timer).
    pub fn show(&mut self, cards: &[PillCard]) {
        objc2::rc::autoreleasepool(|_| {
            let Some(mtm) = MainThreadMarker::new() else {
                log::debug!("hud show skipped: not on the main thread");
                return;
            };
            let live: std::collections::HashSet<u32> = cards.iter().map(|c| c.display_id).collect();
            self.panels.retain(|id, panel| {
                let keep = live.contains(id);
                if !keep {
                    panel.window.orderOut(None);
                }
                keep
            });
            for card in cards {
                self.show_card(card, mtm);
            }
        });
    }

    /// Order every pill out. Idempotent.
    pub fn hide(&mut self) {
        if self.panels.is_empty() {
            return;
        }
        if MainThreadMarker::new().is_none() {
            log::debug!("hud hide skipped: not on the main thread");
            return;
        }
        for panel in self.panels.values() {
            panel.window.orderOut(None);
        }
    }

    fn show_card(&mut self, card: &PillCard, mtm: MainThreadMarker) {
        // A stale daemon display snapshot must never put this pill on a
        // different screen. AppKit's screen list is live on this run loop.
        let center = NSScreen::screens(mtm).iter().find_map(|screen| {
            let description: *mut objc2::runtime::AnyObject =
                unsafe { msg_send![&*screen, deviceDescription] };
            if description.is_null() {
                return None;
            }
            let key = NSString::from_str("NSScreenNumber");
            let number: Option<Retained<NSNumber>> =
                unsafe { msg_send![description, objectForKey: &*key] };
            number.as_ref().and_then(|number| {
                let id: u32 = unsafe { msg_send![number, unsignedIntValue] };
                (id == card.display_id).then(|| {
                    let frame = screen.frame();
                    (
                        frame.origin.x + frame.size.width / 2.0,
                        frame.origin.y + frame.size.height / 2.0,
                    )
                })
            })
        });
        let Some(center) = center else {
            if let Some(panel) = self.panels.remove(&card.display_id) {
                panel.window.orderOut(None);
            }
            return;
        };
        let Some(panel) = self.panels.get_mut(&card.display_id) else {
            let label = build_label(mtm, &card.label(), card.focused);
            let Some((window, label)) = build_panel(mtm, label, center) else {
                log::warn!("hud panel creation failed for display {}", card.display_id);
                return;
            };
            self.panels
                .insert(card.display_id, HudPanel { window, label });
            return;
        };
        refit_panel(panel, &card.label(), card.focused, center);
        panel.window.orderFrontRegardless();
    }
}

impl Default for HudPanels {
    fn default() -> Self {
        Self::new()
    }
}

/// A monospaced, non-editable label. White on the focused monitor's
/// pill, softened gray on the others — the accent is the color, not a
/// size/weight difference that would retrigger layout between flashes.
fn build_label(mtm: MainThreadMarker, text: &str, focused: bool) -> Retained<NSTextField> {
    let title = NSString::from_str(text);
    let label = NSTextField::labelWithString(&title, mtm);
    let font = NSFont::monospacedSystemFontOfSize_weight(FONT_SIZE, 0.0);
    label.setFont(Some(&font));
    let color = if focused {
        NSColor::whiteColor()
    } else {
        NSColor::colorWithWhite_alpha(0.65, 1.0)
    };
    label.setTextColor(Some(&color));
    label.sizeToFit();
    label
}

/// The pill's frame: `size` centered on the display whose usable-area
/// center in AppKit space is `center` (bottom-left origin).
fn pill_frame(width: f64, height: f64, center: (f64, f64)) -> NSRect {
    NSRect::new(
        NSPoint::new(center.0 - width / 2.0, center.1 - height / 2.0),
        NSSize::new(width, height),
    )
}

fn refit_panel(panel: &mut HudPanel, text: &str, focused: bool, center: (f64, f64)) {
    let label_text = NSString::from_str(text);
    panel.label.setStringValue(&label_text);
    // The focused accent lives in the label color, not the window —
    // refresh it, since the same panel may switch roles between flashes.
    let color = if focused {
        NSColor::whiteColor()
    } else {
        NSColor::colorWithWhite_alpha(0.65, 1.0)
    };
    panel.label.setTextColor(Some(&color));
    panel.label.sizeToFit();
    let text_size = panel.label.frame().size;
    let width = text_size.width + PADDING_X * 2.0;
    let height = text_size.height + PADDING_Y * 2.0;
    panel
        .label
        .setFrameOrigin(NSPoint::new(PADDING_X, PADDING_Y));
    // The window resizes its content view to fill on setFrame, so the
    // effect view (and the rounded clipping) tracks the new capsule.
    // No display flag: the next run-loop pass draws it — pills flash,
    // they don't animate. Re-centered per size so a shorter text never
    // leaves the pill off-center.
    panel
        .window
        .setFrame_display(pill_frame(width, height, center), false);
}

/// Create (or finalize) the window for one pill.
fn build_panel(
    mtm: MainThreadMarker,
    label: Retained<NSTextField>,
    center: (f64, f64),
) -> Option<(Retained<HudWindow>, Retained<NSTextField>)> {
    let text_size = label.frame().size;
    let width = text_size.width + PADDING_X * 2.0;
    let height = text_size.height + PADDING_Y * 2.0;

    let effect = NSVisualEffectView::new(mtm);
    effect.setMaterial(NSVisualEffectMaterial::HUDWindow);
    effect.setState(NSVisualEffectState::Active);
    effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    // Layer-backed view → round corners. `NSVisualEffectView` carries no
    // `setCornerRadius` binding here, so the backing layer gets it via
    // one targeted message; `setMasksToBounds` clips the effect view's
    // own drawing to the same radius.
    effect.setWantsLayer(true);
    let layer: Option<Retained<objc2::runtime::AnyObject>> = unsafe { msg_send![&effect, layer] };
    if let Some(layer) = layer {
        unsafe {
            let _: () = msg_send![&layer, setCornerRadius: CORNER_RADIUS];
            let _: () = msg_send![&layer, setMasksToBounds: true];
        }
    }
    effect.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(width, height),
    ));

    label.setFrameOrigin(NSPoint::new(PADDING_X, PADDING_Y));
    effect.addSubview(&label);

    let frame = pill_frame(width, height, center);
    let window: Retained<HudWindow> = unsafe {
        let allocated = HudWindow::alloc(mtm);
        msg_send![allocated, initWithContentRect: frame, styleMask: NSWindowStyleMask::Borderless, backing: NSBackingStoreType::Buffered, defer: false]
    };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(false);
    window.setLevel(WINDOW_LEVEL);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary,
    );
    window.setIgnoresMouseEvents(true);
    window.setContentView(Some(&effect));
    window.orderFrontRegardless();
    Some((window, label.clone()))
}

/// The main-thread tick state: panels plus the last applied card set.
/// The timer skips identical applications — without it, every 30 ms
/// beat would re-fit text and re-place frames for a wholly idle HUD.
struct DrainState {
    panels: HudPanels,
    applied: Option<Vec<PillCard>>,
    geometry: Vec<(u32, NSRect)>,
}

/// The main-thread tick: what the repeating NSTimer runs. Drains the
/// shared slot into panel updates; identical card sets are skipped, so
/// live display identity and geometry are checked each beat. The interval
/// bounds pill latency; the renderer expires the latest flash locally.
pub fn start_main_thread_drain(slot: HudSlotRef) {
    let Some(_) = MainThreadMarker::new() else {
        log::warn!("hud drain not started: not on the main thread");
        return;
    };
    // The panels hold MainThreadOnly ObjC retains: no Arc/Mutex across
    // threads, no promises the types can't keep. The drain state is
    // owned by THIS block — which lives (and is called) only on the
    // main thread, through the main-thread NSTimer — and the RefCell
    // documents the single-owner single-thread access. The cross-thread
    // leg is the HudSlot alone, whose cards are plain data.
    let state_ref = Rc::new(RefCell::new(DrainState {
        panels: HudPanels::new(),
        applied: None,
        geometry: Vec::new(),
    }));
    let slot_for_block = Arc::clone(&slot);
    let state_for_block = Rc::clone(&state_ref);
    // The generated `+timerWithTimeInterval:repeats:block:`
    // signature takes the timer by `NonNull<NSTimer>` — the pointer is
    // only ever used to schedule, never to inspect.
    let block: RcBlock<dyn Fn(core::ptr::NonNull<NSTimer>)> =
        RcBlock::new(move |_timer: core::ptr::NonNull<NSTimer>| {
            let mtm = MainThreadMarker::new().expect("HUD timer runs on main thread");
            let geometry: Vec<_> = NSScreen::screens(mtm)
                .iter()
                .map(|screen| {
                    let description: *mut objc2::runtime::AnyObject =
                        unsafe { msg_send![&*screen, deviceDescription] };
                    let key = NSString::from_str("NSScreenNumber");
                    let number: Option<Retained<NSNumber>> =
                        unsafe { msg_send![description, objectForKey: &*key] };
                    let id: u32 = number
                        .map(|number| unsafe { msg_send![&*number, unsignedIntValue] })
                        .unwrap_or(0);
                    (id, screen.frame())
                })
                .collect();
            // Prune this flash permanently: a replug must wait for a new request.
            // Hold the slot lock through pruning and cloning so a newer flash
            // cannot be overwritten by an older snapshot.
            let latest = {
                let mut slot = slot_for_block.lock().unwrap();
                slot.retain_displays(&geometry.iter().map(|(id, _)| *id).collect::<Vec<_>>());
                slot.clone()
            };
            let mut state = state_for_block.borrow_mut();
            match latest.visible_cards(Instant::now()) {
                Some(cards) => {
                    if state.applied.as_deref() != Some(cards) || state.geometry != geometry {
                        state.geometry = geometry;
                        state.applied = Some(cards.to_vec());
                        state.panels.show(cards);
                    }
                }
                _ => {
                    if state.applied.is_some() {
                        state.applied = None;
                        state.panels.hide();
                    }
                }
            }
        });
    unsafe {
        let timer = NSTimer::timerWithTimeInterval_repeats_block(0.03, true, &block);
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
    }
}
