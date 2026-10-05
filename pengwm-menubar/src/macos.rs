use std::sync::{Arc, Mutex};

use objc2::rc::Retained;
use objc2::runtime::{ProtocolObject, Sel};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSStatusBar,
    NSVariableStatusItemLength,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};

use pengwm_core::command::{BarState, Command};
use pengwm_core::ipc::send_command;

pub struct MenuTargetIvars {
    state: Arc<Mutex<Option<BarState>>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuTargetIvars]
    #[name = "PengwmMenuTarget"]
    struct MenuTarget;

    // SAFETY: NSObjectProtocol imposes no runtime requirements.
    unsafe impl NSObjectProtocol for MenuTarget {}

    // SAFETY: NSMenuDelegate is a pure optional-method protocol; we implement
    // only menuWillOpen:, whose signature matches below.
    unsafe impl NSMenuDelegate for MenuTarget {
        // SAFETY: matches the generated `menuWillOpen:` selector.
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, menu: &NSMenu) {
            rebuild_menu(menu, &self.ivars().state, self);
        }
    }

    impl MenuTarget {
        // SAFETY: matches `switchWorkspace:` — the action selector attached to
        // workspace rows; the sender's tag carries the workspace id.
        #[unsafe(method(switchWorkspace:))]
        fn switch_workspace(&self, sender: &NSMenuItem) {
            let id = sender.tag() as u32;
            log::debug!("menubar switching to workspace {id}");
            if let Err(e) = send_command(&Command::Workspace { id }) {
                log::warn!("menubar workspace switch failed: {e}");
            }
        }

        // SAFETY: matches `quitPengwm:` — the action selector attached to the
        // Quit row. Asks the daemon to shut itself down (which deregisters
        // the launchd job — quit stays quit), then terminates this menubar
        // so the whole app stops together.
        #[unsafe(method(quitPengwm:))]
        fn quit_pengwm(&self, sender: &NSMenuItem) {
            let mtm = self.mtm();
            log::info!("menubar quit — shutting down daemon");
            match send_command(&Command::Quit) {
                Ok(_) => {}
                // Daemon unreachable or wedged (a hung event loop never
                // replies and the IPC socket has no client-side timeout):
                // the whole app should still die. Deregister the launchd
                // job first (nothing respawns us), then SIGTERM and — if
                // the daemon survived even that — SIGKILL, so the icon
                // cannot quit while the daemon limps on.
                Err(e) => {
                    log::warn!("menubar quit command failed, killing daemon: {e}");
                    let _ = std::process::Command::new("sh")
                        .args([
                            "-c",
                            "launchctl bootout gui/$(id -u)/com.pengwm.daemon 2>/dev/null || true",
                        ])
                        .status();
                    let _ = std::process::Command::new("killall")
                        .args(["-TERM", "pengwm"])
                        .status();
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    let _ = std::process::Command::new("killall")
                        .args(["-KILL", "pengwm"])
                        .status();
                }
            }
            NSApplication::sharedApplication(mtm).terminate(Some(sender));
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker, state: Arc<Mutex<Option<BarState>>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MenuTargetIvars { state });
        unsafe { msg_send![super(this), init] }
    }
}

/// Enter the menubar app loop. Runs forever on the main thread.
pub fn run(state: Arc<Mutex<Option<BarState>>>) {
    let mtm = MainThreadMarker::new().expect("run() must be called on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Accessory);

    let target = MenuTarget::new(mtm, state);

    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);

    let menu = NSMenu::new(mtm);
    menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    menu.setAutoenablesItems(false);
    rebuild_menu(&menu, &target.ivars().state, &target);
    item.setMenu(Some(&menu));

    if let Some(button) = item.button(mtm) {
        if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("square.grid.2x2"),
            Some(&NSString::from_str("PengWM workspaces")),
        ) {
            image.setTemplate(true);
            button.setImage(Some(&image));
        } else {
            log::warn!("SF Symbol image unavailable; using text glyph");
            button.setTitle(&NSString::from_str("\u{2638}"));
        }
        button.setToolTip(Some(&NSString::from_str("PengWM workspaces")));
    }

    NSApplication::sharedApplication(mtm).run();
}

fn rebuild_menu(menu: &NSMenu, state: &Mutex<Option<BarState>>, target: &MenuTarget) {
    let mtm = target.mtm();
    menu.removeAllItems();

    let state = state.lock().unwrap().clone();
    let Some(state) = state else {
        add_placeholder(menu, mtm, "Daemon not running");
        add_quit_item(menu, mtm, target);
        return;
    };
    if state.workspaces.is_empty() {
        add_placeholder(menu, mtm, "No workspaces");
        add_quit_item(menu, mtm, target);
        return;
    }

    let switch = Sel::register(c"switchWorkspace:");

    for (i, ws) in state.workspaces.iter().enumerate() {
        let id = i as u32 + 1;

        let title = if ws.active {
            format!("{} \u{2713}", ws.name)
        } else {
            ws.name.clone()
        };
        let row = NSMenuItem::new(mtm);
        row.setTitle(&NSString::from_str(&title));
        row.setTag(id as isize);
        row.setToolTip(Some(&NSString::from_str(&format!(
            "{} window{}",
            ws.window_count,
            if ws.window_count == 1 { "" } else { "s" }
        ))));
        unsafe {
            row.setTarget(Some(target));
            row.setAction(Some(switch));
        }
        menu.addItem(&row);

        if ws.windows.is_empty() {
            let empty = NSMenuItem::new(mtm);
            empty.setTitle(&NSString::from_str("(empty)"));
            empty.setEnabled(false);
            empty.setIndentationLevel(1);
            menu.addItem(&empty);
        } else {
            for app in &ws.windows {
                let row = NSMenuItem::new(mtm);
                row.setTitle(&NSString::from_str(app));
                row.setEnabled(false);
                row.setIndentationLevel(1);
                menu.addItem(&row);
            }
        }
    }

    add_quit_item(menu, mtm, target);
}

fn add_quit_item(menu: &NSMenu, mtm: MainThreadMarker, target: &MenuTarget) {
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let quit = NSMenuItem::new(mtm);
    // The menubar's Quit is the app-level quit: it stops the daemon (and
    // deregisters the launchd job), then exits itself.
    quit.setTitle(&NSString::from_str("Quit PengWM"));
    let quit_sel = Sel::register(c"quitPengwm:");
    unsafe {
        quit.setTarget(Some(target));
        quit.setAction(Some(quit_sel));
    }
    menu.addItem(&quit);
}

fn add_placeholder(menu: &NSMenu, mtm: MainThreadMarker, text: &str) {
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(text));
    item.setEnabled(false);
    menu.addItem(&item);
}
