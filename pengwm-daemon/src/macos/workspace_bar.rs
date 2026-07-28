use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::MainThreadMarker;
use objc2_app_kit::*;
use objc2_foundation::NSPoint;
use objc2_foundation::NSRect;
use objc2_foundation::NSSize;
use objc2_foundation::NSString;

pub struct WorkspaceBarItem {
    pub name: String,
    pub active: bool,
}

pub struct WorkspaceBar {
    window: Option<Retained<NSPanel>>,
    vis_effect: Option<Retained<NSVisualEffectView>>,
    labels: Vec<Retained<NSTextField>>,
    visible: bool,
}

impl Default for WorkspaceBar {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceBar {
    pub fn new() -> Self {
        Self {
            window: None,
            vis_effect: None,
            labels: Vec::new(),
            visible: false,
        }
    }

    pub fn update(&mut self, items: &[WorkspaceBarItem], display_width: f64, display_height: f64) {
        if items.is_empty() {
            self.hide();
            return;
        }

        if self.window.is_none() {
            self.create_panel(display_width, display_height);
        }

        self.rebuild_labels(items, display_width);

        if !self.visible {
            unsafe {
                let _: () = msg_send![
                    self.window.as_ref().unwrap(),
                    orderFront: Option::<&NSObject>::None
                ];
            }
            self.visible = true;
        }
    }

    pub fn hide(&mut self) {
        if self.visible {
            if let Some(ref w) = self.window {
                unsafe {
                    let _: () = msg_send![w, orderOut: Option::<&NSObject>::None];
                }
            }
            self.visible = false;
        }
    }

    fn add_label(
        vis_effect: &NSVisualEffectView,
        text: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        bold: bool,
    ) -> Retained<NSTextField> {
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let tf: Retained<NSTextField> = unsafe {
            msg_send![
                mtm.alloc(),
                initWithFrame: NSRect {
                    origin: NSPoint { x, y },
                    size: NSSize { width, height },
                }
            ]
        };
        let _: () = unsafe { msg_send![&*tf, setBezeled: false] };
        let _: () = unsafe { msg_send![&*tf, setBordered: false] };
        let _: () = unsafe { msg_send![&*tf, setDrawsBackground: false] };
        let _: () = unsafe { msg_send![&*tf, setEditable: false] };
        let _: () = unsafe { msg_send![&*tf, setSelectable: false] };

        let ns_str = NSString::from_str(text);
        let _: () = unsafe { msg_send![&*tf, setStringValue: &*ns_str] };

        let font = if bold {
            NSFont::boldSystemFontOfSize(13.0)
        } else {
            NSFont::systemFontOfSize(13.0)
        };
        let _: () = unsafe { msg_send![&*tf, setFont: &*font] };

        let color = if bold {
            NSColor::whiteColor()
        } else {
            NSColor::colorWithRed_green_blue_alpha(0.6, 0.6, 0.6, 1.0)
        };
        let _: () = unsafe { msg_send![&*tf, setTextColor: &*color] };

        vis_effect.addSubview(&tf);
        tf
    }

    fn rebuild_labels(&mut self, items: &[WorkspaceBarItem], display_width: f64) {
        for label in self.labels.drain(..) {
            let _: () = unsafe { msg_send![&*label, removeFromSuperview] };
        }

        let bar_height: f64 = 30.0;
        let bar_width: f64 = (display_width - 40.0).min(600.0);
        let content_width = bar_width - 20.0;
        let total_items = items.len() as f64;
        let item_width = (content_width / total_items).max(60.0);
        let start_x = 10.0;

        if let Some(ref vis) = self.vis_effect {
            for (i, item) in items.iter().enumerate() {
                let text = if item.active {
                    format!("● {}", item.name)
                } else {
                    format!("○ {}", item.name)
                };
                let label = Self::add_label(
                    vis,
                    &text,
                    start_x + i as f64 * item_width,
                    0.0,
                    item_width,
                    bar_height,
                    item.active,
                );
                self.labels.push(label);
            }
        }
    }

    fn create_panel(&mut self, display_width: f64, _display_height: f64) {
        let bar_height: f64 = 30.0;
        let bar_width: f64 = (display_width - 40.0).min(600.0);

        let x = (display_width - bar_width) / 2.0;
        let y = _display_height - bar_height - 8.0;
        let rect = NSRect {
            origin: NSPoint { x, y },
            size: NSSize {
                width: bar_width,
                height: bar_height,
            },
        };

        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let window: Retained<NSPanel> = unsafe {
            msg_send![
                mtm.alloc(),
                initWithContentRect: rect,
                styleMask: 0,
                backing: 2u32,
                defer: false
            ]
        };

        let _: () = unsafe { msg_send![&*window, setLevel: NSStatusWindowLevel] };
        let _: () = unsafe { msg_send![&*window, setOpaque: false] };
        let bg = NSColor::colorWithRed_green_blue_alpha(0.1, 0.1, 0.1, 0.75);
        let _: () = unsafe { msg_send![&*window, setBackgroundColor: &*bg] };
        let _: () = unsafe { msg_send![&*window, setHidesOnDeactivate: false] };
        let _: () = unsafe { msg_send![&*window, setIgnoresMouseEvents: true] };
        let collection: u64 = (1 << 0) | (1 << 2) | (1 << 8);
        let _: () = unsafe { msg_send![&*window, setCollectionBehavior: collection] };

        let vis_effect: Retained<NSVisualEffectView> = unsafe {
            msg_send![
                mtm.alloc(),
                initWithFrame: NSRect {
                    origin: NSPoint { x: 0.0, y: 0.0 },
                    size: NSSize { width: bar_width, height: bar_height },
                }
            ]
        };
        #[allow(deprecated)]
        vis_effect.setMaterial(NSVisualEffectMaterial::Dark);
        vis_effect.setState(NSVisualEffectState::Active);
        vis_effect.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);

        let content_view: Retained<NSView> = unsafe { msg_send![&*window, contentView] };
        content_view.addSubview(&vis_effect);

        self.window = Some(window);
        self.vis_effect = Some(vis_effect);
    }
}
