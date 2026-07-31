#[cfg(target_os = "macos")]
pub fn set_accessory_activation_policy() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let app = NSApplication::sharedApplication(unsafe { MainThreadMarker::new_unchecked() });
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

#[cfg(not(target_os = "macos"))]
pub fn set_accessory_activation_policy() {}

/// The major macOS version (e.g. `26` for Tahoe). `0` on non-macOS.
#[cfg(target_os = "macos")]
pub fn macos_major_version() -> u32 {
    use objc2_foundation::NSProcessInfo;
    let version = NSProcessInfo::processInfo().operatingSystemVersion();
    version.majorVersion as u32
}

#[cfg(not(target_os = "macos"))]
pub fn macos_major_version() -> u32 {
    0
}
