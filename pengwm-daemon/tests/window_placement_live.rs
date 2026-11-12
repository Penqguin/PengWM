#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pengwm_core::command::{Command, DaemonResponse, WorkspaceInfo};
use pengwm_daemon::adapter::{ObserverRegistry, OsAdapter, WindowListing};
use pengwm_daemon::adapter_macos::MacOsAdapter;
use pengwm_daemon::bar_server::spawn_bar_server_with_path;
use pengwm_daemon::config::keybinds::KeybindConfig;
use pengwm_daemon::macos::ax_element;
use pengwm_daemon::prefix::{PrefixConfig, PrefixKey};
use pengwm_daemon::state::StateManager;
use tokio::sync::mpsc;

struct Fixture(Child);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_fixture(root: &std::path::Path) -> (Fixture, i32) {
    let path = root.join("fixture.swift");
    std::fs::write(&path, r#"
import AppKit
import Darwin
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
var windows: [NSWindow] = []
for index in 0..<3 {
    let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 480, height: 320),
        styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
    window.title = "PengWM placement fixture \(index + 1)"
    window.setAccessibilityIdentifier("pengwm-placement-fixture-\(index)")
    window.level = .floating
    window.collectionBehavior = [.fullScreenPrimary]
    window.isReleasedWhenClosed = false
    window.makeKeyAndOrderFront(nil)
    windows.append(window)
}
class FixtureActions: NSObject {
    @objc func normalWindows(_ sender: Any?) {
        for window in NSApp.windows { window.level = .normal }
    }
}
let actions = FixtureActions()
let menu = NSMenu()
let appItem = NSMenuItem(title: "PengWM Validation", action: nil, keyEquivalent: "")
appItem.submenu = NSMenu(title: "PengWM Validation")
menu.addItem(appItem)
let view = NSMenuItem(title: "View", action: nil, keyEquivalent: "")
let submenu = NSMenu(title: "View")
let fullscreen = NSMenuItem(title: "Toggle Full Screen", action: #selector(NSWindow.toggleFullScreen(_:)), keyEquivalent: "f")
fullscreen.keyEquivalentModifierMask = [.command, .control]
submenu.addItem(fullscreen)
let normal = NSMenuItem(title: "Use Normal Window Level", action: #selector(FixtureActions.normalWindows(_:)), keyEquivalent: "")
normal.target = actions
submenu.addItem(normal)
view.submenu = submenu
menu.addItem(view)
app.finishLaunching()
app.mainMenu = menu
DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
    print(ProcessInfo.processInfo.processIdentifier)
    fflush(stdout)
}
app.run()
"#).unwrap();
    let bundle = root.join("PengWM Validation.app");
    let executable = bundle.join("Contents/MacOS/PengWMValidation");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(bundle.join("Contents/Info.plist"), r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>org.pengwm.validation</string>
<key>CFBundleExecutable</key><string>PengWMValidation</string><key>CFBundleName</key><string>PengWM Validation</string>
<key>CFBundlePackageType</key><string>APPL</string><key>LSUIElement</key><true/></dict></plist>"#).unwrap();
    assert!(std::process::Command::new("swiftc")
        .arg(path)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap()
        .success());
    let child = std::process::Command::new(executable)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut fixture = Fixture(child);
    let mut line = String::new();
    BufReader::new(fixture.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let pid = line.trim().parse().expect("fixture must report its PID");
    (fixture, pid)
}

fn query(state: &mut StateManager) -> Vec<WorkspaceInfo> {
    let (tx, mut rx) = mpsc::channel(1);
    state.on_command(Command::QueryState, Some(tx));
    match rx.try_recv().unwrap() {
        DaemonResponse::State { workspaces } => workspaces,
        response => panic!("expected workspace state, got {response:?}"),
    }
}

/// Interactive mode keeps the real placement engine attached only to the
/// fixture's events. Snapshot/control files coordinate the human cable checks
/// and CUA gestures while the installed daemon continues serving user windows.
fn monitor_hardware(
    state: &mut StateManager,
    pid: i32,
    ids: &[u64],
    root: &std::path::Path,
    events: &mut mpsc::Receiver<pengwm_daemon::event_loop::DaemonEvent>,
) {
    use pengwm_daemon::event_loop::DaemonEvent;
    use serde_json::json;
    let control = root.join("control");
    let mut recent_events = Vec::new();
    let mut previous = String::new();
    let start = std::time::Instant::now();
    eprintln!("HARDWARE_MONITOR={}", root.display());
    while start.elapsed() < Duration::from_secs(1800) {
        unsafe {
            core_foundation::runloop::CFRunLoopRunInMode(
                core_foundation::runloop::kCFRunLoopDefaultMode,
                0.05,
                1,
            );
        }
        while let Ok(event) = events.try_recv() {
            let description = format!("{event:?}");
            match event {
                DaemonEvent::WindowHidden(id) if ids.contains(&id) => state.on_window_hidden(id),
                DaemonEvent::WindowShown(id) if ids.contains(&id) => state.on_window_shown(id),
                DaemonEvent::WindowFocused(id) if ids.contains(&id) => state.on_window_focused(id),
                DaemonEvent::WindowDestroyed(id) if ids.contains(&id) => {
                    state.on_window_destroyed(id)
                }
                _ => continue,
            }
            recent_events.push(description);
            if recent_events.len() > 32 {
                recent_events.remove(0);
            }
        }
        state.on_tick();
        let mut window_info = Vec::new();
        for (element, id) in unsafe { ax_element::windows_for_pid(pid) }.unwrap_or_default() {
            if !ids.contains(&id) {
                continue;
            }
            let hidden = unsafe { ax_element::bool_attribute(element.ptr(), "AXMinimized") };
            let fullscreen = unsafe { ax_element::bool_attribute(element.ptr(), "AXFullScreen") };
            if hidden {
                state.on_window_hidden(id);
            } else if fullscreen {
                state.on_window_focused(id);
            } else {
                state.on_window_shown(id);
                state.on_fullscreen_exit(id);
            }
            let rect = unsafe { ax_element::get_window_rect(element.ptr()) };
            window_info.push(json!({"id": id, "hidden": hidden, "fullscreen": fullscreen,
                "rect": rect.map(|r| json!({"x":r.x,"y":r.y,"width":r.width,"height":r.height}))}));
        }
        if let Ok(command) = std::fs::read_to_string(&control) {
            std::fs::remove_file(&control).unwrap();
            let command = command.trim();
            if command == "stop" {
                break;
            }
            if let Some(id) = command
                .strip_prefix("workspace-")
                .and_then(|s| s.parse().ok())
            {
                state.on_command(Command::Workspace { id }, None);
            } else if command == "cycle-window" {
                state.on_command(Command::CycleWindow { backward: false }, None);
            }
            recent_events.push(format!("Control({command})"));
        }
        let spaces: Vec<_> = pengwm_daemon::macos::spaces::snapshot()
            .unwrap_or_default()
            .iter()
            .map(|s| {
                json!({"display":s.display_id,"space":s.current_space,"type":s.current_type,
                "first":s.first_user_space,"allowed":s.on_first_desktop()})
            })
            .collect();
        let displays: Vec<_> = pengwm_daemon::macos::cg_display::active_displays()
            .iter()
            .map(|d| json!({"id":d.id,"origin":d.origin,"size":d.size}))
            .collect();
        let snapshot = json!({"pid":pid,"root":root,"spaces":spaces,"displays":displays,
            "workspaces":query(state),"windows":window_info,"events":recent_events});
        let serialized = serde_json::to_string_pretty(&snapshot).unwrap();
        if serialized != previous {
            state
                .save_session(&root.join("observed-session.toml"))
                .unwrap();
            std::fs::write(root.join("snapshot.tmp"), &serialized).unwrap();
            std::fs::rename(root.join("snapshot.tmp"), root.join("snapshot.json")).unwrap();
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(root.join("history.jsonl"))
                .and_then(|mut file| {
                    use std::io::Write;
                    writeln!(file, "{}", serde_json::to_string(&snapshot).unwrap())
                })
                .unwrap();
            previous = serialized;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "requires Accessibility, a GUI session, and Swift for dedicated test windows"]
fn native_overflow_carries_and_settles() {
    assert!(
        ax_element::is_process_trusted(),
        "Accessibility permission required"
    );
    // A dedicated fixture protects existing user windows from this test's
    // manual lifecycle events. Floating CG level also keeps the running
    // installed daemon from tiling the fixture concurrently.
    let root = std::env::temp_dir().join(format!("pengwm-placement-live-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pengwm")).unwrap();
    let (_fixture, pid) = start_fixture(&root);
    std::fs::write(
        root.join("pengwm/config.toml"),
        r#"
max_tiles = 1
restore_last_session = false
[windows]
float_level_overlays = false
float_apps = []
[menubar]
enabled = false
[[workspaces]]
name = "Fixture A"
[[workspaces]]
name = "Fixture B"
[[workspaces]]
name = "Fixture C"
"#,
    )
    .unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &root);
    std::env::set_var("XDG_STATE_HOME", &root);
    let (events_tx, mut events_rx) = mpsc::channel(512);
    let mut adapter = MacOsAdapter::with_callback(Box::new(move |event| {
        let _ = events_tx.try_send(event);
    }));
    adapter.attach_observer(pid);
    let mut ids = match adapter.poll_windows_for_pid(pid) {
        WindowListing::Answered(ids) => ids,
        WindowListing::NoAnswer => panic!("fixture did not answer Accessibility listing"),
    };
    ids.sort_unstable();
    assert_eq!(ids.len(), 3, "fixture must own exactly three windows");
    for id in &ids {
        assert!(adapter
            .window_title(*id)
            .is_some_and(|title| title.starts_with("PengWM placement fixture ")));
        assert_eq!(
            adapter.window_kind(*id),
            Some(pengwm_daemon::adapter::WindowClass::Untracked)
        );
    }
    let (tx, _rx) = mpsc::channel(512);
    let excluded: Vec<_> = pengwm_daemon::macos::ns_workspace::running_pids_any_policy()
        .into_iter()
        .filter(|other| *other != pid)
        .collect();
    let bar = spawn_bar_server_with_path(root.join("bar.sock").to_str().unwrap());
    let mut state = StateManager::new(
        tx,
        Arc::new(Mutex::new(KeybindConfig::default())),
        Arc::new(Mutex::new(PrefixKey::new(PrefixConfig::default()))),
        Box::new(adapter),
        bar,
        excluded.clone(),
    );
    for id in &ids {
        state.on_window_created(*id, pid);
    }
    std::thread::sleep(Duration::from_millis(200));
    let initial = query(&mut state);
    assert_eq!(
        initial.iter().map(|ws| ws.window_count).collect::<Vec<_>>(),
        [3, 0, 0]
    );
    let read_rect = |id| unsafe {
        let windows = ax_element::windows_for_pid(pid).expect("fixture listing");
        let (element, _) = windows.iter().find(|(_, wid)| *wid == id).unwrap();
        ax_element::get_window_rect(element.ptr()).expect("fixture rectangle")
    };
    let tile = read_rect(ids[0]);
    let overlay = read_rect(ids[2]);
    assert!(
        tile.width > overlay.width * 1.2 && tile.height > overlay.height * 1.2,
        "tile {tile:?}, overflow overlay {overlay:?}"
    );
    if std::env::var_os("PENGWM_HARDWARE_VALIDATION").is_some() {
        monitor_hardware(&mut state, pid, &ids, &root, &mut events_rx);
        for id in ids {
            state.on_window_destroyed(id);
        }
        std::fs::remove_dir_all(root).unwrap();
        return;
    }
    state.on_command(Command::CycleWindow { backward: false }, None);
    assert_eq!(query(&mut state)[0].focused_window, Some(ids[0]));
    state.on_command(Command::Workspace { id: 2 }, None);
    std::thread::sleep(Duration::from_millis(200));
    let second = query(&mut state);
    assert_eq!(
        second.iter().map(|ws| ws.window_count).collect::<Vec<_>>(),
        [1, 2, 0]
    );
    let display = pengwm_daemon::macos::cg_display::active_displays()
        .into_iter()
        .find(|display| display.id == second[1].monitor_id)
        .unwrap();
    let landed = read_rect(ids[1]);
    assert!(
        landed.x >= display.origin.0 as f64
            && landed.x < (display.origin.0 + display.size.0 as i32) as f64,
        "carried window must actually land on destination display: {landed:?}"
    );
    assert!(
        landed.width > display.size.0 as f64 * 0.85,
        "carried window must actually tile: {landed:?}"
    );
    state.on_command(Command::Workspace { id: 3 }, None);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        query(&mut state)
            .iter()
            .map(|ws| ws.window_count)
            .collect::<Vec<_>>(),
        [1, 1, 1]
    );
    state.on_command(Command::Workspace { id: 1 }, None);
    assert_eq!(
        query(&mut state)
            .iter()
            .map(|ws| ws.window_count)
            .collect::<Vec<_>>(),
        [1, 1, 1]
    );
    let session_path = root.join("pengwm/state.toml");
    state.save_session(&session_path).unwrap();
    let saved = pengwm_daemon::state::session::load_from(&session_path).unwrap();
    assert_eq!(
        saved.live_windows.len(),
        3,
        "fixture AXIdentifiers must support verified recovery"
    );
    drop(state);
    let config_path = root.join("pengwm/config.toml");
    let config = std::fs::read_to_string(&config_path).unwrap().replace(
        "restore_last_session = false",
        "restore_last_session = true",
    );
    std::fs::write(config_path, config).unwrap();
    let (tx, _rx) = mpsc::channel(512);
    let bar = spawn_bar_server_with_path(root.join("bar-restart.sock").to_str().unwrap());
    let mut state = StateManager::new(
        tx,
        Arc::new(Mutex::new(KeybindConfig::default())),
        Arc::new(Mutex::new(PrefixKey::new(PrefixConfig::default()))),
        Box::new(MacOsAdapter::with_callback(Box::new(|_| {}))),
        bar,
        excluded,
    );
    assert_eq!(
        query(&mut state)
            .iter()
            .map(|ws| ws.window_count)
            .collect::<Vec<_>>(),
        [1, 1, 1],
        "verified homes must survive a real adapter restart"
    );
    // Replay the late geometry drift observed after the physical hotplug.
    // Periodic production recovery must repair the OS readback, not just homes.
    let fault_adapter = MacOsAdapter::with_callback(Box::new(|_| {}));
    let _ = fault_adapter.poll_windows_for_pid(pid);
    let displaced = pengwm_core::layout::Rect::new(
        tile.x + 150.0,
        tile.y + 180.0,
        tile.width * 0.7,
        tile.height * 0.7,
    );
    assert!(fault_adapter
        .set_window_rect(ids[0], displaced, false)
        .is_ok());
    assert!((read_rect(ids[0]).y - tile.y).abs() > 100.0);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        state.on_tick();
        let actual = read_rect(ids[0]);
        if (actual.x - tile.x).abs() <= 3.0
            && (actual.y - tile.y).abs() <= 3.0
            && (actual.width - tile.width).abs() <= 3.0
            && (actual.height - tile.height).abs() <= 3.0
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "periodic recovery failed: actual={actual:?}, target={tile:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    for id in ids {
        state.on_window_destroyed(id);
    }
    std::fs::remove_dir_all(root).unwrap();
}
