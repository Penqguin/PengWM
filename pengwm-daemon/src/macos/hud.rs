//! The transient workspace HUD executor (Spec 0001).
//!
//! One borderless, click-through pill per monitor: dark translucent
//! rounded rect, monospaced `N · Workspace` text, dead-center. The
//! focused monitor's pill is accented; the prefix-armed pill carries a
//! `»` marker. Panels are created lazily per display, restyled in place
//! per flash, and ordered out on hide — the daemon process owns them
//! (no child process, no IPC; see the spec's rejected alternatives).
//!
//! Positioning is CG-derived, deliberately not `NSScreen`: after a
//! display connects while the daemon runs, AppKit's screen list can lag
//! the CG list (`hud card dropped: display N has no matching NSScreen`
//! was the hotplug-era symptom), while the daemon tiler itself is
//! CG-bound. One coordinate-space conversion applies — AppKit global
//! space is bottom-left origin, AX/CG space is the main display's
//! top-left with y down: `y_appkit = main_cg_height - y_ax`. The
//! formula is machinery-free and verifiable against `NSScreen.frame`
//! on the same machine (see the module docs of `cg_display`).
//!
//! Pure planning (what the cards say, which monitor, which is focused)
//! lives in `state::hud`. This module only knows AppKit: how to put a
//! rounded translucent capsule with text on each screen.

use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFont, NSScreen, NSTextField, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSRect, NSPoint, NSSize, NSString};

use crate::adapter::HudCard;
use crate::macos::cg_display;

/// Comfortable text inset inside the capsule (each side).
const PADDING_X: f64 = 14.0;
const PADDING_Y: f64 = 7.0;
/// Pill placement: dead-center of the screen — the one spot you cannot
/// miss, and precisely the "where did things land" answer the HUD
/// exists for. Its bulge is click-through and 500 ms, so covering
/// mid-screen content momentarily is the point, not a hazard.
const CORNER_RADIUS: f64 = 9.0;
const FONT_SIZE: f64 = 12.0;
/// The floating window level (3): above every normal/floating window,
/// below menus — the HUD answers "what is showing", never competes with
/// one. Combined with `CanJoinAllSpaces` the pill also overlays native
/// fullscreen workspaces.
const WINDOW_LEVEL: isize = 3;

struct HudPanel {
    window: Retained<HudWindow>,
    label: Retained<NSTextField>,
}

define_class!(
    /// An `NSWindow` that refuses to be constrained into whatever
    /// AppKit's cached screen list knows about. The daemon runs its own
    /// CFRunLoop; AppKit's screen cache inside this process can lag a
    /// hotplug indefinitely — the observed effect was a pill ordered
    /// for the freshly re-plugged monitor (CG id 8, alive in the
    /// daemon's own `active_displays()`) being clamped to the primary
    /// screen's corner, i.e. on-screen nowhere near its display. The
    /// one method AppKit consults for placement is
    /// `constrainFrameRect:toScreen:`; passing the frame through
    /// unchanged makes the window land exactly where the CG-derived
    /// math put it, AppKit's screen list notwithstanding. Window
    /// placement is confirmed by the window server, which *does* know
    /// every screen, so nothing visual depends on AppKit's cache.
    #[unsafe(super = NSWindow)]
    #[thread_kind = MainThreadOnly]
    #[name = "PengwmHudWindow"]
    struct HudWindow;

    // SAFETY: NSObjectProtocol imposes no runtime requirements.
    unsafe impl NSObjectProtocol for HudWindow {}

    impl HudWindow {
        // SAFETY: matches `constrainFrameRect:toScreen:` — the hook
        // AppKit calls to validate a window frame against a screen.
        // Returning the frame unchanged is the entire point of the
        // subclass: our frames are CG-derived and already correct.
        #[unsafe(method(constrainFrameRect:toScreen:))]
        unsafe fn constrain_frame_rect(
            &self,
            frame_rect: NSRect,
            _screen: Option<&NSScreen>,
        ) -> NSRect {
            frame_rect
        }
    }
);

/// Panels keyed by CG display id. Created lazily by `show`; dropped
/// when a display leaves the live set. All methods require the main
/// thread (`MainThreadMarker::new()`); off-thread callers are
/// logged and skipped — the window system is not there to ask.
pub struct HudPanels {
    panels: HashMap<u32, HudPanel>,
}

impl HudPanels {
    pub fn new() -> Self {
        Self {
            panels: HashMap::new(),
        }
    }

    /// Draw every card: existing panels are restyled and repositioned,
    /// missing ones are created, panels whose display vanished are
    /// dropped. The batch is the whole frame — the executor never
    /// diff-tracks pill text.
    ///
    /// The whole FFI pass is one autorelease pool: AppKit's helper
    /// objects here (screens array, description dicts, label/text
    /// temps, the frames AppKit computes behind setStringValue/
    /// sizeToFit/setFrameDisplay) would otherwise autoreleases into the
    /// daemon's pool-less CFRunLoop thread and outlive the flash — a
    /// few KB per flash, forever. Same trade `ns_workspace::
    /// screen_top_insets` already makes ("the screens enumeration
    /// autoreleases on the daemon's pool-less CFRunLoop thread").
    pub fn show(&mut self, cards: &[HudCard]) {
        objc2::rc::autoreleasepool(|_| {
            let Some(mtm) = MainThreadMarker::new() else {
                log::debug!("hud show skipped: not on the main thread");
                return;
            };
            let frames = display_frames();
            let live: std::collections::HashSet<u32> =
                frames.iter().map(|(id, _)| *id).collect();
            self.panels.retain(|id, _| live.contains(id));
            for card in cards {
                let Some((_, center)) = frames
                    .iter()
                    .find(|(id, _)| *id == card.display_id)
                else {
                    log::debug!(
                        "hud card dropped: display {} is not in the CG display list",
                        card.display_id
                    );
                    continue;
                };
                self.show_card(card, *center, mtm);
            }
        });
    }

    /// Order every pill out. Idempotent — hiding hidden windows is a
    /// no-op, and the panels stay reusable for the next flash.
    pub fn hide(&mut self) {
        if self.panels.is_empty() {
            return;
        }
        if MainThreadMarker::new().is_none() {
            // The single-threaded event loop is the only caller in prod;
            // this is the same off-thread guard `show` logs.
            log::debug!("hud hide skipped: not on the main thread");
            return;
        }
        // Pool, for the same reason `show` is: orderOut touches AppKit
        // internals that autorelease on the pool-less thread.
        objc2::rc::autoreleasepool(|_| {
            for panel in self.panels.values() {
                panel.window.orderOut(None);
            }
        });
    }

    fn show_card(&mut self, card: &HudCard, center_hint: (f64, f64), mtm: MainThreadMarker) {
        let Some(panel) = self.panels.get_mut(&card.display_id) else {
            let label = build_label(mtm, &card.label(), card.focused);
            let Some((window, label)) = build_panel(mtm, label, center_hint) else {
                log::warn!("hud panel creation failed for display {}", card.display_id);
                return;
            };
            self.panels.insert(
                card.display_id,
                HudPanel {
                    window,
                    label,
                },
            );
            return;
        };
        refit_panel(panel, &card.label(), card.focused, center_hint);
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

/// Restyle an existing panel for one card: text, accent color, and the
/// capsule's size+position. Pill text differs between almost every
/// flash (armed marker, swap moves, the `—` slot) — sizing once at
/// creation would clip longer text and leave a drifted capsule after
/// shorter text (the panel was centered for the *old* width).
/// The re-fit mirrors `build_panel`'s sizing math exactly.
fn pill_frame(width: f64, height: f64, center_ax: (f64, f64)) -> NSRect {
    // The only coordinates AppKit will accept for window frames: global
    // bottom-left-origin. The display center arrives in AX space
    // (top-left origin, y down); one flip per the module-doc formula.
    let (cx, cy_ax) = center_ax;
    NSRect::new(
        NSPoint::new(
            cx - width / 2.0,
            cg_display::raw_main_height() - cy_ax - height / 2.0,
        ),
        NSSize::new(width, height),
    )
}

fn refit_panel(panel: &mut HudPanel, text: &str, focused: bool, center_ax: (f64, f64)) {
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
    // they don't animate. The frame's anchor is re-centered per size so
    // a shorter text never leaves the pill off-center.
    panel
        .window
        .setFrame_display(pill_frame(width, height, center_ax), false);
}

/// Create (or finalize) the window for one pill: rounded translucent
/// capsule content sized to the label, centered on the matching screen.
/// Returns the window and the label to keep updating.
fn build_panel(
    mtm: MainThreadMarker,
    label: Retained<NSTextField>,
    center_ax: (f64, f64),
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
    let layer: Option<Retained<objc2::runtime::AnyObject>> =
        unsafe { msg_send![&effect, layer] };
    if let Some(layer) = layer {
        unsafe {
            let _: () = msg_send![&layer, setCornerRadius: CORNER_RADIUS];
            let _: () = msg_send![&layer, setMasksToBounds: true];
        }
    }
    effect.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)));

    label.setFrameOrigin(NSPoint::new(PADDING_X, PADDING_Y));
    effect.addSubview(&label);

    let frame = pill_frame(width, height, center_ax);
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

/// (CG display id, AX-space center of the display's usable area) per
/// display — straight from the same `active_displays()` snapshot the
/// tiler is built on, so pill placement and tile placement can never
/// disagree about which monitors exist. Notably not `NSScreen`: the
/// AppKit screen list lags CG after a hotplug (the "second monitor
/// shows no pill" report).

/// (CG display id, AX-space center of the display's usable area) per
/// display — straight from the same `active_displays()` snapshot the
/// tiler is built on, so pill placement and tile placement can never
/// disagree about which monitors exist. Notably not `NSScreen`: the
/// AppKit screen list lags CG after a hotplug (the "second monitor
/// shows no pill" report).
fn display_frames() -> Vec<(u32, (f64, f64))> {
    cg_display::active_displays()
        .iter()
        .map(|d| {
            let (w, h) = (d.size.0 as f64, d.size.1 as f64);
            (
                d.id,
                (d.origin.0 as f64 + w / 2.0, d.origin.1 as f64 + h / 2.0),
            )
        })
        .collect()
}
