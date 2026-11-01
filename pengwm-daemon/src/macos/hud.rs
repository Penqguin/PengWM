//! The transient workspace HUD executor (Spec 0001).
//!
//! One borderless, click-through pill per monitor: dark translucent
//! rounded rect, monospaced `N · Workspace` text, dead-center. The
//! focused monitor's pill is accented; the prefix-armed pill carries a
//! `»` marker. Panels are created lazily per display, restyled in place
//! per flash, and ordered out on hide — the daemon process owns them
//! (no child process, no IPC; see the spec's rejected alternatives).
//!
//! Pure planning (what the cards say, which monitor, which is focused)
//! lives in `state::hud`. This module only knows AppKit: how to put a
//! rounded translucent capsule with text on each screen.

use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::MainThreadOnly as _;
use objc2::{msg_send, MainThreadMarker};
use objc2_app_kit::{
    NSScreen, NSBackingStoreType, NSColor, NSFont, NSTextField, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    NSString,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSNumber};

use crate::adapter::HudCard;

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
    window: Retained<NSWindow>,
    label: Retained<NSTextField>,
}

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
    pub fn show(&mut self, cards: &[HudCard]) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::debug!("hud show skipped: not on the main thread");
            return;
        };
        let screens = screen_frames(mtm);
        let live: std::collections::HashSet<u32> =
            screens.iter().map(|(id, _)| *id).collect();
        self.panels.retain(|id, _| live.contains(id));
        for card in cards {
            let Some((_, frame)) = screens
                .iter()
                .find(|(id, _)| *id == card.display_id)
            else {
                log::debug!(
                    "hud card dropped: display {} has no matching NSScreen",
                    card.display_id
                );
                continue;
            };
            self.show_card(card, *frame, mtm);
        }
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
        for panel in self.panels.values() {
            panel.window.orderOut(None);
        }
    }

    fn show_card(&mut self, card: &HudCard, screen: NSRect, mtm: MainThreadMarker) {
        let Some(panel) = self.panels.get_mut(&card.display_id) else {
            let label = build_label(mtm, &card.label(), card.focused);
            let Some((window, label)) = build_panel(mtm, label, screen) else {
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
        refit_panel(panel, &card.label(), card.focused, screen);
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
fn pill_frame(width: f64, height: f64, screen: NSRect) -> NSRect {
    NSRect::new(
        NSPoint::new(
            screen.origin.x + (screen.size.width - width) / 2.0,
            screen.origin.y + (screen.size.height - height) / 2.0,
        ),
        NSSize::new(width, height),
    )
}

fn refit_panel(panel: &mut HudPanel, text: &str, focused: bool, screen: NSRect) {
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
    panel.window.setFrame_display(pill_frame(width, height, screen), false);
}

/// Create (or finalize) the window for one pill: rounded translucent
/// capsule content sized to the label, centered on the matching screen.
/// Returns the window and the label to keep updating.
fn build_panel(
    mtm: MainThreadMarker,
    label: Retained<NSTextField>,
    screen: NSRect,
) -> Option<(Retained<NSWindow>, Retained<NSTextField>)> {
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

    let frame = pill_frame(width, height, screen);
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
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

/// (CG display id, AppKit-global frame) per screen — the
/// `NSScreenNumber` device-description mapping the codebase already
/// uses for menu-bar insets (`ns_workspace::screen_top_insets`). AppKit
/// global frames are bottom-left-origin; the executor positions in that
/// space directly, never converting through the AX (top-left) space.
/// Raw `msg_send` like that helper: `NSDictionary`/`NSPredicate` trait
/// features are not part of the daemon's foundation feature set this
/// link can pay for.
fn screen_frames(mtm: MainThreadMarker) -> Vec<(u32, NSRect)> {
    let mut out = Vec::new();
    let screens = NSScreen::screens(mtm);
    for screen in screens.iter() {
        let desc: Retained<objc2::runtime::NSObject> =
            unsafe { msg_send![&screen, deviceDescription] };
        let key = NSString::from_str("NSScreenNumber");
        let value: Option<Retained<NSNumber>> = unsafe {
            msg_send![&desc, objectForKey: &*key]
        };
        let Some(num) = value else { continue };
        let id: u32 = unsafe { msg_send![&num, unsignedIntValue] };
        let frame: NSRect = unsafe { msg_send![&screen, frame] };
        out.push((id, frame));
    }
    out
}
