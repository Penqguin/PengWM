use std::sync::{Arc, Mutex};
use std::process::Stdio;

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

use crate::{TrustedState, UpdateState};

/// How often the trust poller re-checks Accessibility trust while the daemon
/// is unreachable. Cheap: a single FFI call, no UI work.
const TRUST_POLL: std::time::Duration = std::time::Duration::from_secs(2);

// SAFETY: `AXIsProcessTrusted` is HIServices' documented zero-arg function;
// ApplicationServices (and so HIServices) is already linked transitively by
// AppKit. It returns 0 or 1, which is a valid C `bool`.
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

pub struct MenuTargetIvars {
    state: Arc<Mutex<Option<BarState>>>,
    trusted: TrustedState,
    update: UpdateState,
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
            let ivars = self.ivars();
            rebuild_menu(menu, &ivars.state, &ivars.trusted, &ivars.update, self);
        }
    }

    impl MenuTarget {
        // SAFETY: matches `openAccessibility:` — the action selector attached
        // to the "needs permission" row shown while the daemon is unreachable
        // and this process lacks Accessibility trust. Deep-links System
        // Settings to the Accessibility pane, which modern macOS silently
        // remaps to Privacy & Security → Accessibility.
        #[unsafe(method(openAccessibility:))]
        fn open_accessibility(&self, _sender: &NSMenuItem) {
            log::info!("menubar opening System Settings → Accessibility");
            if let Err(e) = std::process::Command::new("/usr/bin/open")
                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
                .spawn()
            {
                log::warn!("opening Accessibility settings failed: {e}");
            }
        }

        // SAFETY: matches `runUpdateNow:` — the action selector attached to
        // the "Update now…" row shown next to the update-available label.
        // Detaches the daemon binary's own `pengwm update` subcommand so the
        // menubar never blocks or waits on it.
        #[unsafe(method(runUpdateNow:))]
        fn run_update_now(&self, _sender: &NSMenuItem) {
            // Same candidate-path pattern the daemon uses to find this
            // menubar: the sibling next to ourselves first (bare-binary and
            // bundle layouts both colocate the two binaries), else let
            // $PATH resolve `pengwm`.
            let bin = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("pengwm")))
                .filter(|path| path.is_file())
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| "pengwm".to_string());
            log::info!("menubar launching {bin} update");
            let child = std::process::Command::new(bin)
                .arg("update")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if let Err(e) = child {
                log::warn!("launching pengwm update failed: {e}");
            }
        }

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
    fn new(
        mtm: MainThreadMarker,
        state: Arc<Mutex<Option<BarState>>>,
        trusted: TrustedState,
        update: UpdateState,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MenuTargetIvars {
            state,
            trusted,
            update,
        });
        unsafe { msg_send![super(this), init] }
    }
}

/// Enter the menubar app loop. Runs forever on the main thread.
pub fn run(state: Arc<Mutex<Option<BarState>>>, trusted: TrustedState, update: UpdateState) {
    let mtm = MainThreadMarker::new().expect("run() must be called on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Accessory);

    spawn_trust_watcher(Arc::clone(&state), Arc::clone(&trusted));

    let target = MenuTarget::new(mtm, state, trusted, update);

    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);

    let menu = NSMenu::new(mtm);
    menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    menu.setAutoenablesItems(false);
    rebuild_menu(&menu, &target.ivars().state, &target.ivars().trusted, &target.ivars().update, &target);
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

fn rebuild_menu(menu: &NSMenu, state: &Mutex<Option<BarState>>, trusted: &TrustedState, update: &UpdateState, target: &MenuTarget) {
    let mtm = target.mtm();
    menu.removeAllItems();

    add_update_items(menu, mtm, target, update);

    let Some(state) = state.lock().unwrap().clone() else {
        add_disconnected_items(menu, mtm, target, trusted);
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

/// Bottom of the trust poll in the v1 grant flow: while the daemon is
/// unreachable, pulse Accessibility trust so the menu can flip between a
/// "needs permission" row and a plain "still connecting" row on the next
/// open. Polled ONLY while disconnected — once the daemon connects the poller
/// pauses and connection.rs's snapshot drives the menu back to the normal
/// state, so this degrades gracefully in both directions.
fn spawn_trust_watcher(state: Arc<Mutex<Option<BarState>>>, trusted: TrustedState) {
    std::thread::spawn(move || loop {
        if state.lock().unwrap().is_none() {
            let ok = unsafe { AXIsProcessTrusted() };
            *trusted.lock().unwrap() = ok;
        }
        std::thread::sleep(TRUST_POLL);
    });
}

/// The disconnected surface (replaces the old bare "Daemon not running"
/// placeholder). The daemon is (re)started forever by launchd KeepAlive, so
/// once trust is granted the system heals itself with no further user action;
/// hence the enabled deep-link row plus the disabled "it's trying" row.
fn add_disconnected_items(
    menu: &NSMenu,
    mtm: MainThreadMarker,
    target: &MenuTarget,
    trusted: &TrustedState,
) {
    if !*trusted.lock().unwrap() {
        let needs = NSMenuItem::new(mtm);
        needs.setTitle(&NSString::from_str("PengWM needs Accessibility \u{26A0}"));
        let sel = Sel::register(c"openAccessibility:");
        unsafe {
            needs.setTarget(Some(target));
            needs.setAction(Some(sel));
        }
        menu.addItem(&needs);
    }
    let starting = NSMenuItem::new(mtm);
    starting.setTitle(&NSString::from_str("Starting PengWM\u{2026}"));
    starting.setEnabled(false);
    menu.addItem(&starting);
}

/// Update-available surface when update_check.rs saw a strictly newer
/// release: a disabled informational label naming the version, then an
/// enabled row that kicks off `pengwm update` via the installed app bundle.
fn add_update_items(menu: &NSMenu, mtm: MainThreadMarker, target: &MenuTarget, update: &UpdateState) {
    let Some(tag) = update.lock().unwrap().clone() else {
        return;
    };
    let info = NSMenuItem::new(mtm);
    info.setTitle(&NSString::from_str(&format!(
        "Update available: {tag} \u{2192} run `pengwm update`"
    )));
    info.setEnabled(false);
    menu.addItem(&info);

    let now = NSMenuItem::new(mtm);
    now.setTitle(&NSString::from_str("Update now\u{2026}"));
    let sel = Sel::register(c"runUpdateNow:");
    unsafe {
        now.setTarget(Some(target));
        now.setAction(Some(sel));
    }
    menu.addItem(&now);

    menu.addItem(&NSMenuItem::separatorItem(mtm));
}
