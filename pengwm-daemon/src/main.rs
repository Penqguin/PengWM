use std::sync::{Arc, Mutex};
use std::thread;

use pengwm_daemon::config;
use pengwm_daemon::macos;
use pengwm_daemon::event_loop;
use pengwm_daemon::ipc_server;

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSApplicationDidFinishLaunchingNotification};
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_foundation::NSNotificationCenter;

fn main() {
    env_logger::init();

    #[cfg(target_os = "macos")]
    {
        if !macos::ax_element::is_process_trusted() {
            eprintln!("PengWM needs Accessibility permissions to control windows.");
            eprintln!("Opening System Settings…\n");

            macos::ax_element::request_trusted_access();

            let start = std::time::Instant::now();
            let timeout = std::time::Duration::from_secs(120);
            loop {
                if macos::ax_element::is_process_trusted() {
                    eprintln!("\n✓ Permission granted. Starting PengWM…");
                    break;
                }
                if start.elapsed() > timeout {
                    eprintln!("\nTimed out waiting for Accessibility permission.");
                    eprintln!("Grant it manually:");
                    eprintln!("  System Settings → Privacy & Security → Accessibility");
                    eprintln!("Then re-run the daemon.");
                    std::process::exit(1);
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }

    eprintln!("[1/6] Loading config…");
    let keybinds = Arc::new(Mutex::new(config::keybinds::KeybindConfig::load()));

    eprintln!("[2/6] Initializing event loop and state…");
    let (mut event_loop, tx) = event_loop::EventLoop::new(Arc::clone(&keybinds));

    #[cfg(target_os = "macos")]
    {
        eprintln!("[3/6] Starting global keybind tap…");
        macos::event_tap::start(tx.clone(), Arc::clone(&keybinds));

        eprintln!("[4/6] Attaching app lifecycle observers…");
        macos::ns_workspace::observe(tx.clone());

        eprintln!("[5/6] Registering display hotplug callback…");
        macos::cg_display::register_hotplug_callback(tx.clone());

        eprintln!("[5.5/6] Initializing NSApplication as accessory…");
        let app = NSApplication::sharedApplication(unsafe { MainThreadMarker::new_unchecked() });
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        let center = NSNotificationCenter::defaultCenter();
        unsafe {
            center.postNotificationName_object(&NSApplicationDidFinishLaunchingNotification, Some(&app));
        }
    }

    eprintln!("[6/6] Starting IPC server and config watcher…");
    config::watcher::watch(tx.clone());

    // Spawn the UDS listener on a background thread.
    thread::spawn(move || {
        ipc_server::start_ipc_server(tx);
    });

    eprintln!("PengWM daemon ready (pid {})", std::process::id());

    // Run the event loop synchronously on this thread.
    // Drains macOS events (AXObserver, CGEventTap, NSWorkspace) and mpsc messages.
    event_loop.run_sync();
}
