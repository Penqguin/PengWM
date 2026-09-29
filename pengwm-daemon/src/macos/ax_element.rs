use std::ffi::c_void;
use std::ptr;

use accessibility_sys::*;
use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::{CFEqual, CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::boolean::kCFBooleanTrue;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};

use pengwm_core::layout::{rects_close, Rect, WriteOutcome, LAYOUT_EPSILON};
use pengwm_core::tree::WindowId;

pub fn is_process_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub fn is_process_trusted_with_prompt() -> bool {
    unsafe {
        let key = CFString::new("AXTrustedCheckOptionPrompt");
        let val = CFNumber::from(1i32);
        let dict = CFDictionary::from_CFType_pairs(&[(key, val)]);
        AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef())
    }
}

/// Show the system Accessibility permission prompt dialog.
/// macOS will open System Settings so the user can grant access.
pub fn request_trusted_access() {
    let _ = is_process_trusted_with_prompt();
}

/// Upper bound on how long one AX request may block the daemon.
///
/// Every AX call is a synchronous mach RPC into the target app's main
/// thread, issued from the one thread that also pumps the CGEventTap
/// (keybinds) and the AX observers. At the system default, an app that
/// stops servicing its run loop — Firefox right after wake, mid
/// session-restore — stalls the whole window manager for seconds per
/// call, and a tap callback that can't run in time gets the tap disabled
/// by the OS (`kCGEventTapDisabledByTimeout`), dropping keystrokes.
///
/// A healthy round trip is well under a millisecond, so this leaves
/// orders of magnitude of headroom while bounding the stall. Being wrong
/// is cheap and self-healing: a timed-out request surfaces as
/// `kAXErrorCannotComplete`, which the write path already classifies as
/// `Transient` and retries on the next layout or 2s sweep.
///
/// This is the one dial. Deliberately tighter than yabai's 1.0s, because
/// the CGEventTap callback must also run within roughly a second or the
/// OS disables the tap. Raise it if `focus_window`'s `kAXRaiseAction`
/// starts missing on heavy apps — that is the first thing a too-tight
/// value would break.
pub const MESSAGING_TIMEOUT_SECS: f32 = 0.25;

/// Set the process-wide default AX messaging timeout. Elements without
/// their own value inherit it, which covers the window elements handed to
/// us inside `kAXWindows` arrays as well as the app elements we create.
/// Belt and braces with `apply_messaging_timeout` on the hot paths.
pub fn set_global_messaging_timeout(seconds: f32) {
    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            log::warn!(
                "AXUIElementCreateSystemWide returned null — AX calls keep the system default timeout"
            );
            return;
        }
        let err = AXUIElementSetMessagingTimeout(system_wide, seconds);
        CFRelease(system_wide as CFTypeRef);
        if err == kAXErrorSuccess {
            log::info!("AX messaging timeout set to {}s", seconds);
        } else {
            log::warn!(
                "AXUIElementSetMessagingTimeout(system-wide, {}s) failed: {}",
                seconds,
                error_string(err)
            );
        }
    }
}

/// Apply the daemon's messaging timeout to one element.
///
/// # Safety
/// `element` must be a valid, retained `AXUIElementRef` (null is a no-op).
pub unsafe fn apply_messaging_timeout(element: AXUIElementRef) {
    if element.is_null() {
        return;
    }
    let err = AXUIElementSetMessagingTimeout(element, MESSAGING_TIMEOUT_SECS);
    if err != kAXErrorSuccess {
        log::debug!(
            "AXUIElementSetMessagingTimeout({}s) failed: {}",
            MESSAGING_TIMEOUT_SECS,
            error_string(err)
        );
    }
}

/// Create an app-level AX element with the messaging timeout already
/// applied. The single seam for app elements, so no call site can
/// silently inherit the system default.
///
/// # Safety
/// `pid` must reference a running process. The caller owns the returned
/// element and must `CFRelease` it.
pub unsafe fn create_app_element(pid: i32) -> AXUIElementRef {
    let app = AXUIElementCreateApplication(pid);
    apply_messaging_timeout(app);
    app
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn _AXUIElementGetWindow(element: AXUIElementRef, window_id: *mut u32) -> AXError;
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller is responsible for
/// ensuring the element remains valid for the duration of the call.
pub unsafe fn ax_window_id_from_element(element: AXUIElementRef) -> Option<WindowId> {
    let mut wid: u32 = 0;
    if _AXUIElementGetWindow(element, &mut wid) == kAXErrorSuccess {
        Some(wid as WindowId)
    } else {
        None
    }
}

/// # Safety
/// `element` must be a valid, retained `AXUIElementRef`.
pub unsafe fn set_window_position_raw(
    element: AXUIElementRef,
    x: f64,
    y: f64,
) -> anyhow::Result<()> {
    let pos_name = CFString::new(kAXPositionAttribute);
    let mut point = CGPoint { x, y };
    let pos_value = AXValueCreate(kAXValueTypeCGPoint, &mut point as *mut _ as *mut c_void);
    if pos_value.is_null() {
        anyhow::bail!("AXValueCreate failed for position");
    }
    let err = AXUIElementSetAttributeValue(
        element,
        pos_name.as_concrete_TypeRef(),
        pos_value as CFTypeRef,
    );
    CFRelease(pos_value as CFTypeRef);
    if err != kAXErrorSuccess {
        anyhow::bail!(
            "AXUIElementSetAttributeValue position error: {}",
            error_string(err)
        );
    }
    Ok(())
}

/// # Safety
/// `element` must be a valid, retained `AXUIElementRef`.
pub unsafe fn set_window_size_raw(
    element: AXUIElementRef,
    width: f64,
    height: f64,
) -> anyhow::Result<()> {
    let size_name = CFString::new(kAXSizeAttribute);
    let mut size = CGSize { width, height };
    let size_value = AXValueCreate(kAXValueTypeCGSize, &mut size as *mut _ as *mut c_void);
    if size_value.is_null() {
        anyhow::bail!("AXValueCreate failed for size");
    }
    let err = AXUIElementSetAttributeValue(
        element,
        size_name.as_concrete_TypeRef(),
        size_value as CFTypeRef,
    );
    CFRelease(size_value as CFTypeRef);
    if err != kAXErrorSuccess {
        anyhow::bail!(
            "AXUIElementSetAttributeValue size error: {}",
            error_string(err)
        );
    }
    Ok(())
}

/// # Safety
/// `element` must be a valid, retained `AXUIElementRef`.
pub unsafe fn set_window_rect(element: AXUIElementRef, rect: Rect) -> WriteOutcome {
    // Firefox and other non-native apps often ignore the first AX write or
    // shift position when size changes (frame-vs-content ordering). Do
    // position/size/position per attempt with a readback, up to 3 attempts.
    // A persistent drift must NOT report Ok: callers record `applied_rects`
    // on success and skip forever, which strands login-time Firefox windows
    // (session-restore / not-yet-resizable) at the wrong rect until the app
    // is fully quit + reopened. Report Drift instead so the write retries
    // on the next layout / 2s sweep.
    //
    // EPS is the shared `LAYOUT_EPSILON`: a window inside it is "at target"
    // everywhere else, so demanding tighter here only buys a perpetual
    // 3-attempt storm for windows with a few-px frame offset. Genuinely
    // misplaced windows (tens/hundreds of px) still report Drift and retry.
    //
    // STABLE_EPS detects a pinned window: readbacks within 2px across
    // attempts mean zero real progress (sub-pixel jitter at most), so a
    // third attempt cannot converge and reports Pinned early. The caller
    // backs off writes for pinned windows instead of storming them;
    // shifting windows (progress between attempts) keep the full 3 attempts
    // and the shifting Drift report.
    const ATTEMPTS: u32 = 3;
    const EPS: f64 = LAYOUT_EPSILON;
    const STABLE_EPS: f64 = 2.0;
    let mut last_actual: Option<Rect> = None;
    for attempt in 0..ATTEMPTS {
        if let Err(e) = set_window_position_raw(element, rect.x, rect.y) {
            return WriteOutcome::Transient(e.to_string());
        }
        if let Err(e) = set_window_size_raw(element, rect.width, rect.height) {
            return WriteOutcome::Transient(e.to_string());
        }
        // Size can shift position — re-assert it.
        if let Err(e) = set_window_position_raw(element, rect.x, rect.y) {
            return WriteOutcome::Transient(e.to_string());
        }
        match get_window_rect(element) {
            Some(actual) if rects_close(actual, rect, EPS) => return WriteOutcome::Ok,
            Some(actual) => {
                // The first miss is the cheapest moment to ask whether this
                // window accepts geometry writes at all. Native fullscreen,
                // a zoomed window, or a genuinely fixed-size one reports
                // non-settable and will never converge — two more attempts
                // buy nothing but eight more round trips and eight more
                // reflows. Report it futile and let the pin backoff own the
                // retry cadence. Checked after attempt 1, not before, so
                // healthy windows never pay for the query.
                if attempt == 0 {
                    let pos_settable = is_attribute_settable(element, kAXPositionAttribute);
                    let size_settable = is_attribute_settable(element, kAXSizeAttribute);
                    if !pos_settable || !size_settable {
                        log::debug!(
                            "set_window_rect: window refuses geometry writes (position settable={}, size settable={}) target {:?} actual {:?} — not retrying",
                            pos_settable,
                            size_settable,
                            rect,
                            actual
                        );
                        return WriteOutcome::Pinned {
                            target: rect,
                            actual,
                        };
                    }
                }
                if let Some(prev) = last_actual {
                    if rects_close(prev, actual, STABLE_EPS) {
                        log::debug!(
                            "set_window_rect pinned attempt {}/{} target {:?} actual {:?} (unchanged since last attempt, bailing)",
                            attempt + 1,
                            ATTEMPTS,
                            rect,
                            actual
                        );
                        return WriteOutcome::Pinned {
                            target: rect,
                            actual,
                        };
                    }
                }
                log::debug!(
                    "set_window_rect drift attempt {}/{} target {:?} actual {:?}",
                    attempt + 1,
                    ATTEMPTS,
                    rect,
                    actual
                );
                last_actual = Some(actual);
            }
            None => return WriteOutcome::Ok,
        }
    }
    match last_actual {
        Some(actual) => WriteOutcome::Drift {
            target: rect,
            actual,
        },
        None => WriteOutcome::Ok,
    }
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller must ensure the
/// element remains valid for the duration of the call.
pub unsafe fn get_window_rect(element: AXUIElementRef) -> Option<Rect> {
    let pos_name = CFString::new(kAXPositionAttribute);
    let size_name = CFString::new(kAXSizeAttribute);

    let mut pos_val: CFTypeRef = ptr::null();
    let err_pos =
        AXUIElementCopyAttributeValue(element, pos_name.as_concrete_TypeRef(), &mut pos_val);
    if err_pos != kAXErrorSuccess || pos_val.is_null() {
        return None;
    }

    let mut size_val: CFTypeRef = ptr::null();
    let err_size =
        AXUIElementCopyAttributeValue(element, size_name.as_concrete_TypeRef(), &mut size_val);
    if err_size != kAXErrorSuccess || size_val.is_null() {
        CFRelease(pos_val);
        return None;
    }

    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    AXValueGetValue(
        pos_val as AXValueRef,
        kAXValueTypeCGPoint,
        &mut point as *mut _ as *mut c_void,
    );
    AXValueGetValue(
        size_val as AXValueRef,
        kAXValueTypeCGSize,
        &mut size as *mut _ as *mut c_void,
    );

    CFRelease(pos_val);
    CFRelease(size_val);

    Some(Rect {
        x: point.x,
        y: point.y,
        width: size.width,
        height: size.height,
    })
}

/// Activate the owning application so its windows come to the front.
fn activate_app(pid: i32) -> bool {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    match NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        Some(app) => app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows),
        None => false,
    }
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller must ensure the
/// element is valid for the duration of the call and that `pid` references a running
/// process with Accessibility permissions.
pub unsafe fn focus_window(element: AXUIElementRef, pid: i32) {
    activate_app(pid);

    let raise_name = CFString::new(kAXRaiseAction);
    AXUIElementPerformAction(element, raise_name.as_concrete_TypeRef());

    let main_name = CFString::new(kAXMainAttribute);
    AXUIElementSetAttributeValue(
        element,
        main_name.as_concrete_TypeRef(),
        kCFBooleanTrue as CFTypeRef,
    );

    let app = create_app_element(pid);
    if app.is_null() {
        return;
    }
    let focused_name = CFString::new(kAXFocusedWindowAttribute);
    AXUIElementSetAttributeValue(
        app,
        focused_name.as_concrete_TypeRef(),
        element as CFTypeRef,
    );
    CFRelease(app as CFTypeRef);
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller must ensure the
/// element is valid and that the Accessibility API can be called safely.
pub unsafe fn is_manageable(element: AXUIElementRef) -> bool {
    let role_name = CFString::new(kAXRoleAttribute);
    let mut role_val: CFTypeRef = ptr::null();
    let err =
        AXUIElementCopyAttributeValue(element, role_name.as_concrete_TypeRef(), &mut role_val);
    if err != kAXErrorSuccess || role_val.is_null() {
        return false;
    }
    let role_str = CFString::wrap_under_create_rule(role_val as CFStringRef);
    if role_str != kAXWindowRole {
        return false;
    }

    let subrole_name = CFString::new(kAXSubroleAttribute);
    let mut subrole_val: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(
        element,
        subrole_name.as_concrete_TypeRef(),
        &mut subrole_val,
    );
    if err != kAXErrorSuccess || subrole_val.is_null() {
        return false;
    }
    let subrole_str = CFString::wrap_under_create_rule(subrole_val as CFStringRef);
    subrole_str == kAXStandardWindowSubrole
}

/// # Safety
///
/// `pid` must reference a valid running process with Accessibility permissions.
pub unsafe fn focused_window_for_pid(pid: i32) -> Option<WindowId> {
    let app = create_app_element(pid);
    if app.is_null() {
        return None;
    }
    let attr = CFString::new(kAXFocusedWindowAttribute);
    let mut value: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(app, attr.as_concrete_TypeRef(), &mut value);
    CFRelease(app as CFTypeRef);
    if err != kAXErrorSuccess || value.is_null() {
        return None;
    }
    // The value is an AXUIElementRef for the focused window — resolve its
    // window id rather than casting the pointer itself.
    let window_id = ax_window_id_from_element(value as AXUIElementRef);
    CFRelease(value);
    window_id
}

pub fn frontmost_pid() -> Option<i32> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        let ws = NSWorkspace::sharedWorkspace();
        ws.frontmostApplication().map(|app| app.processIdentifier())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// # Safety
///
/// The caller must ensure that `pid` references a valid running process and that
/// the Accessibility API is called from a trusted process with the necessary permissions.
pub unsafe fn windows_for_pid(pid: i32) -> Vec<(AXUIElementRef, WindowId)> {
    let app = create_app_element(pid);
    if app.is_null() {
        return Vec::new();
    }

    let windows_attr = CFString::new(kAXWindowsAttribute);
    let mut windows_array: CFArrayRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(
        app,
        windows_attr.as_concrete_TypeRef(),
        &mut windows_array as *mut _ as *mut CFTypeRef,
    );

    if err != kAXErrorSuccess || windows_array.is_null() {
        CFRelease(app as CFTypeRef);
        return Vec::new();
    }

    let count = CFArrayGetCount(windows_array);
    let mut result = Vec::new();

    for i in 0..count {
        let elem = CFArrayGetValueAtIndex(windows_array, i) as AXUIElementRef;
        if elem.is_null() {
            continue;
        }
        if !is_manageable(elem) {
            continue;
        }
        if let Some(window_id) = ax_window_id_from_element(elem) {
            CFRetain(elem as CFTypeRef);
            // Window elements are separate AXUIElementRefs from the app
            // element, so they need the timeout applied in their own right.
            apply_messaging_timeout(elem);
            result.push((elem, window_id));
        }
    }

    CFRelease(windows_array as CFTypeRef);
    CFRelease(app as CFTypeRef);

    result
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef` representing a window.
pub unsafe fn close_window(element: AXUIElementRef) {
    let attr = CFString::new("AXCloseButton");
    let mut close_button: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut close_button);
    if err != kAXErrorSuccess || close_button.is_null() {
        log::warn!("close_window: no close button found (err={})", err);
        return;
    }
    let press = CFString::new("AXPress");
    AXUIElementPerformAction(close_button as AXUIElementRef, press.as_concrete_TypeRef());
    CFRelease(close_button);
}

#[repr(C)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
struct CGSize {
    width: f64,
    height: f64,
}

/// Set only the window position (no resize) — fast BottomEdge hide for
/// apps like Firefox where `AXSize 1×1` triggers slow reflow. Used as
/// fallback when `set_window_rect` fails on size.
/// # Safety
/// `element` must be a valid, retained `AXUIElementRef`.
pub unsafe fn set_window_position(element: AXUIElementRef, x: f64, y: f64) -> anyhow::Result<()> {
    let pos_name = CFString::new(kAXPositionAttribute);
    let mut point = CGPoint { x, y };
    let pos_value = AXValueCreate(kAXValueTypeCGPoint, &mut point as *mut _ as *mut c_void);
    if pos_value.is_null() {
        anyhow::bail!("AXValueCreate failed for position");
    }
    let err = AXUIElementSetAttributeValue(
        element,
        pos_name.as_concrete_TypeRef(),
        pos_value as CFTypeRef,
    );
    CFRelease(pos_value as CFTypeRef);
    if err != kAXErrorSuccess {
        anyhow::bail!(
            "AXUIElementSetAttributeValue position error: {}",
            error_string(err)
        );
    }
    Ok(())
}

/// App-level attribute assistive tech sets to ask an app for its full
/// accessibility tree. Not in `accessibility_sys`'s constant set.
const ENHANCED_USER_INTERFACE: &str = "AXEnhancedUserInterface";

/// Suspends `AXEnhancedUserInterface` on an app for the guard's lifetime.
///
/// With it on, Chromium- and Gecko-based apps animate every AX frame
/// change instead of applying it, so a tiling write visibly crawls the
/// window into place. `suspend` reads the flag, turns it off only if it
/// was on, and `Drop` restores it — assistive tech such as VoiceOver
/// depends on it, so it must never be left off. When the flag was already
/// off (the normal case) the guard costs one AX read and does nothing else.
pub struct EnhancedUiGuard {
    app: AXUIElementRef,
    was_enabled: bool,
}

impl EnhancedUiGuard {
    /// # Safety
    /// `pid` must reference a running process.
    pub unsafe fn suspend(pid: i32) -> Self {
        let app = create_app_element(pid);
        let was_enabled = !app.is_null() && bool_attribute(app, ENHANCED_USER_INTERFACE);
        if was_enabled {
            set_bool_attribute(app, ENHANCED_USER_INTERFACE, false);
        }
        Self { app, was_enabled }
    }

    /// Whether the app had enhanced UI on when the guard was taken.
    pub fn was_enabled(&self) -> bool {
        self.was_enabled
    }
}

impl Drop for EnhancedUiGuard {
    fn drop(&mut self) {
        if self.app.is_null() {
            return;
        }
        unsafe {
            if self.was_enabled {
                set_bool_attribute(self.app, ENHANCED_USER_INTERFACE, true);
            }
            CFRelease(self.app as CFTypeRef);
        }
    }
}

/// # Safety
/// `element` must be a valid, retained `AXUIElementRef`.
unsafe fn set_bool_attribute(element: AXUIElementRef, attribute: &str, value: bool) {
    let name = CFString::new(attribute);
    let cf_value = if value {
        kCFBooleanTrue
    } else {
        core_foundation::boolean::kCFBooleanFalse
    };
    let err =
        AXUIElementSetAttributeValue(element, name.as_concrete_TypeRef(), cf_value as CFTypeRef);
    if err != kAXErrorSuccess {
        log::debug!("set {}={} failed: {}", attribute, value, error_string(err));
    }
}

/// Whether the app will accept a write to `attribute` on this element.
///
/// A window reports its position/size as non-settable when it is in
/// native fullscreen, zoomed, genuinely fixed-size, or otherwise not
/// under the app's control right now. Writing anyway costs a full
/// 3-attempt storm — twelve AX round trips and a reflow per write — to
/// achieve nothing, so the writer consults this once the first attempt
/// misses and reports the write futile instead of retrying.
///
/// Unreadable (the app didn't answer) is treated as settable: the write
/// path's existing drift/pin classification is the better judge than a
/// guess made from a failed query.
///
/// # Safety
/// `element` must be a valid, retained `AXUIElementRef`.
pub unsafe fn is_attribute_settable(element: AXUIElementRef, attribute: &str) -> bool {
    let name = CFString::new(attribute);
    let mut settable: std::ffi::c_uchar = 0;
    let err = AXUIElementIsAttributeSettable(element, name.as_concrete_TypeRef(), &mut settable);
    if err != kAXErrorSuccess {
        return true;
    }
    settable != 0
}

/// Read a string-valued attribute (role, subrole, title). `None` when the
/// attribute is absent, unreadable, or not a string.
///
/// # Safety
/// `element` must be a valid, retained `AXUIElementRef`.
pub unsafe fn string_attribute(element: AXUIElementRef, attribute: &str) -> Option<String> {
    let name = CFString::new(attribute);
    let mut value: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(element, name.as_concrete_TypeRef(), &mut value);
    if err != kAXErrorSuccess || value.is_null() {
        return None;
    }
    Some(CFString::wrap_under_create_rule(value as CFStringRef).to_string())
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller must ensure the
/// element remains valid for the duration of the call.
pub unsafe fn bool_attribute(element: AXUIElementRef, attribute: &str) -> bool {
    let name = CFString::new(attribute);
    let mut value: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(element, name.as_concrete_TypeRef(), &mut value);
    if err != kAXErrorSuccess || value.is_null() {
        return false;
    }
    let is_true = CFEqual(value, kCFBooleanTrue as CFTypeRef) != 0;
    CFRelease(value);
    is_true
}

#[cfg(test)]
mod tests {
    // rects_close is canonical in pengwm-core::layout — tested there.
    // AX writer behavior is exercised through TestAdapter outcomes.
}
