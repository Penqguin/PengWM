use pengwm_daemon::config;
use pengwm_daemon::macos;
use pengwm_daemon::event_loop;

fn main() {
    env_logger::init();

    #[cfg(target_os = "macos")]
    {
        if !macos::ax_element::is_process_trusted() {
            eprintln!("PengWM requires Accessibility permissions.");
            eprintln!("Go to System Settings > Privacy & Security > Accessibility");
            eprintln!("and add Terminal (or whatever runs this daemon) to the list.");
            eprintln!("You may need to restart the daemon after granting permission.");
            std::process::exit(1);
        }
    }

    let (_event_loop, tx) = event_loop::EventLoop::new();

    let keybinds = config::keybinds::KeybindConfig::default();

    #[cfg(target_os = "macos")]
    {
        macos::event_tap::start(tx.clone(), keybinds);
        macos::ns_workspace::observe(tx.clone());
        macos::cg_display::register_hotplug_callback(tx);
    }

    log::info!("PengWM daemon started");

    #[cfg(target_os = "macos")]
    unsafe {
        let _run_loop = core_foundation::runloop::CFRunLoopGetCurrent();
        core_foundation::runloop::CFRunLoopRun();
    }

    #[cfg(not(target_os = "macos"))]
    {
        log::warn!("PengWM only runs on macOS. Running in stub mode.");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
}
