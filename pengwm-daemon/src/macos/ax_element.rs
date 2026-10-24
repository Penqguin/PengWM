use std::ffi::c_void;
use std::ptr;

use accessibility_sys::*;
use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::{CFEqual, CFTypeRef, TCFType};
use core_foundation::boolean::kCFBooleanTrue;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};

use pengwm_core::layout::{rects_close, Rect, WriteOutcome, LAYOUT_EPSILON};
use pengwm_core::tree::WindowId;

use super::cf_intern::intern;
use super::cf_ref::CFRef;

pub fn is_process_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Show the system Accessibility permission prompt dialog.
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
        let system_wide = CFRef::from_create(AXUIElementCreateSystemWide());
        let Some(system_wide) = system_wide else {
            log::warn!(
                "AXUIElementCreateSystemWide returned null — AX calls keep the system default timeout"
            );
            return;
        };
        let err = AXUIElementSetMessagingTimeout(system_wide.ptr(), seconds);
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
/// silently inherit the system default. The `CFRef` owns the +1 —
/// releasing is per-scope by construction, never a caller duty.
///
/// # Safety
/// `pid` must reference a running process.
pub unsafe fn create_app_element(pid: i32) -> Option<CFRef<AXUIElementRef>> {
    let app = CFRef::from_create(AXUIElementCreateApplication(pid))?;
    apply_messaging_timeout(app.ptr());
    Some(app)
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
    let pos_name = intern(kAXPositionAttribute);
    let mut point = CGPoint { x, y };
    let Some(pos_value) = CFRef::from_create(AXValueCreate(
        kAXValueTypeCGPoint,
        &mut point as *mut _ as *mut c_void,
    )) else {
        anyhow::bail!("AXValueCreate failed for position");
    };
    let err = AXUIElementSetAttributeValue(
        element,
        pos_name,
        pos_value.ptr() as CFTypeRef,
    );
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
    let size_name = intern(kAXSizeAttribute);
    let mut size = CGSize { width, height };
    let Some(size_value) = CFRef::from_create(AXValueCreate(
        kAXValueTypeCGSize,
        &mut size as *mut _ as *mut c_void,
    )) else {
        anyhow::bail!("AXValueCreate failed for size");
    };
    let err = AXUIElementSetAttributeValue(
        element,
        size_name,
        size_value.ptr() as CFTypeRef,
    );
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
    let pos_name = intern(kAXPositionAttribute);
    let size_name = intern(kAXSizeAttribute);

    let mut pos_raw: CFTypeRef = ptr::null();
    let err_pos =
        AXUIElementCopyAttributeValue(element, pos_name, &mut pos_raw);
    if err_pos != kAXErrorSuccess {
        return None;
    }
    let pos_val = CFRef::from_create(pos_raw)?;

    let mut size_raw: CFTypeRef = ptr::null();
    let err_size =
        AXUIElementCopyAttributeValue(element, size_name, &mut size_raw);
    if err_size != kAXErrorSuccess {
        // pos_val drops here — the failure path releases what it owns.
        return None;
    }
    let size_val = CFRef::from_create(size_raw)?;

    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    AXValueGetValue(
        pos_val.ptr() as AXValueRef,
        kAXValueTypeCGPoint,
        &mut point as *mut _ as *mut c_void,
    );
    AXValueGetValue(
        size_val.ptr() as AXValueRef,
        kAXValueTypeCGSize,
        &mut size as *mut _ as *mut c_void,
    );

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
    objc2::rc::autoreleasepool(|_pool| {
        match NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            Some(app) => app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows),
            None => false,
        }
    })
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller must ensure the
/// element is valid for the duration of the call and that `pid` references a running
/// process with Accessibility permissions.
pub unsafe fn focus_window(element: AXUIElementRef, pid: i32) {
    activate_app(pid);

    let raise_name = intern(kAXRaiseAction);
    AXUIElementPerformAction(element, raise_name);

    let main_name = intern(kAXMainAttribute);
    AXUIElementSetAttributeValue(
        element,
        main_name,
        kCFBooleanTrue as CFTypeRef,
    );

    let Some(app) = create_app_element(pid) else {
        return;
    };
    let focused_name = intern(kAXFocusedWindowAttribute);
    AXUIElementSetAttributeValue(
        app.ptr(),
        focused_name,
        element as CFTypeRef,
    );
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller must ensure the
/// element is valid and that the Accessibility API can be called safely.
pub unsafe fn classify(element: AXUIElementRef) -> crate::adapter::WindowClass {
    let role = string_attribute(element, kAXRoleAttribute);
    let subrole = string_attribute(element, kAXSubroleAttribute);
    crate::adapter::WindowClass::classify_window(role.as_deref(), subrole.as_deref())
}

/// # Safety
///
/// `pid` must reference a valid running process with Accessibility permissions.
pub unsafe fn focused_window_for_pid(pid: i32) -> Option<WindowId> {
    let app = create_app_element(pid)?;
    let attr = intern(kAXFocusedWindowAttribute);
    let mut value: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(app.ptr(), attr, &mut value);
    // `app` drops here whether or not the copy succeeded.
    if err != kAXErrorSuccess || value.is_null() {
        return None;
    }
    // The value is an AXUIElementRef for the focused window — resolve its
    // window id rather than casting the pointer itself. The Copy handed us
    // a +1: `CFRef::from_create` owns it and releases on scope exit.
    CFRef::from_create(value as AXUIElementRef)
        .and_then(|elem| ax_window_id_from_element(elem.ptr()))
}

pub fn frontmost_pid() -> Option<i32> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        objc2::rc::autoreleasepool(|_pool| {
            let ws = NSWorkspace::sharedWorkspace();
            ws.frontmostApplication().map(|app| app.processIdentifier())
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Returns owned element refs (`CFRef`, +1 each) with their window id.
/// Classification is the *caller's* job — the element cache's owner
/// classifies (and level-gates) exactly once per window id, at
/// first sight, and reuses the cached class for ids it already knows;
/// sheets and unknowns are the caller's filter, same fate as the old
/// binary gate. Borrowed candidates whose window id check fails are
/// simply never retained — dropping the vector releases the survivors,
/// and nothing owned is ever left behind.
///
/// Poll an app's windows through AX. The buy side of the `WindowListing`
/// seam: `None` is the no-answer case (the app element could not be
/// created, or the `AXWindows` read errored — the blackout signature);
/// `Some` is an actual answer, empty included (a closed-only-window app's
/// legitimate "I have no windows").
///
/// Each listed window carries the element the poll copied, so the caller
/// can cache it; the window id comes from the private `_AXUIElementGetWindow`
///
/// # Safety
///
/// The caller must ensure that `pid` references a valid running process and that
/// the Accessibility API is called from a trusted process with the necessary permissions.
pub unsafe fn windows_for_pid(pid: i32) -> Option<Vec<(CFRef<AXUIElementRef>, WindowId)>> {
    let app = create_app_element(pid)?;

    let windows_attr = intern(kAXWindowsAttribute);
    let mut windows_array: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(
        app.ptr(),
        windows_attr,
        &mut windows_array as *mut _ as *mut CFTypeRef,
    );
    if err != kAXErrorSuccess {
        // `app` drops here; nothing else was created.
        return None;
    }
    let windows_array = CFRef::from_create(windows_array as CFArrayRef)?;

    let count = CFArrayGetCount(windows_array.ptr());
    let mut result = Vec::new();
    for i in 0..count {
        let elem = CFArrayGetValueAtIndex(windows_array.ptr(), i) as AXUIElementRef;
        if elem.is_null() {
            continue;
        }
        if let Some(window_id) = ax_window_id_from_element(elem) {
            // Get-rule borrow from the array — retain it into ownership.
            let Some(elem) = CFRef::from_get(elem) else {
                continue;
            };
            // Window elements are separate AXUIElementRefs from the app
            // element, so they need the timeout applied in their own right.
            apply_messaging_timeout(elem.ptr());
            result.push((elem, window_id));
        }
    }

    Some(result)
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef` representing a window.
pub unsafe fn close_window(element: AXUIElementRef) {
    let attr = intern("AXCloseButton");
    let mut close_button: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(element, attr, &mut close_button);
    if err != kAXErrorSuccess {
        log::warn!("close_window: no close button found (err={})", err);
        return;
    }
    let Some(close_button) = CFRef::from_create(close_button as AXUIElementRef) else {
        return;
    };
    let press = intern("AXPress");
    AXUIElementPerformAction(close_button.ptr(), press);
}

/// Bring a window to the front of the stacking order (`AXRaise` action).
/// Popups re-assert it on switch-back reveal so overlays return on top;
/// macOS stacking after un-hide is otherwise unspecified.
/// # Safety
/// `element` must be a valid, retained `AXUIElementRef` representing a window.
pub unsafe fn raise_window(element: AXUIElementRef) {
    let action = intern(kAXRaiseAction);
    AXUIElementPerformAction(element, action);
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
    let pos_name = intern(kAXPositionAttribute);
    let mut point = CGPoint { x, y };
    let Some(pos_value) = CFRef::from_create(AXValueCreate(
        kAXValueTypeCGPoint,
        &mut point as *mut _ as *mut c_void,
    )) else {
        anyhow::bail!("AXValueCreate failed for position");
    };
    let err = AXUIElementSetAttributeValue(
        element,
        pos_name,
        pos_value.ptr() as CFTypeRef,
    );
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
///
/// The guard **borrows** its element: the app-element memo in
/// `MacOsAdapter` owns the ref and evicts it only via `forget_pid`, which
/// cannot run while a write is in flight (single-threaded event loop, no
/// reentrancy through the synchronous AX calls) — so the borrowed
/// pointer is valid for the guard's life.
pub struct EnhancedUiGuard {
    element: Option<AXUIElementRef>,
    was_enabled: bool,
}

impl EnhancedUiGuard {
    /// A guard that does nothing: the app element could not be resolved
    /// (the app is unreachable), so there is no flag to read and nothing
    /// to restore. Same answer the per-pid create gave: suspend silently
    /// skips, the write proceeds.
    pub fn inactive() -> Self {
        Self {
            element: None,
            was_enabled: false,
        }
    }

    /// # Safety
    /// `element` must be a valid, retained `AXUIElementRef` that stays
    /// alive for the guard's lifetime (see the struct docs).
    pub unsafe fn suspend(element: AXUIElementRef) -> Self {
        let was_enabled = bool_attribute(element, ENHANCED_USER_INTERFACE);
        if was_enabled {
            set_bool_attribute(element, ENHANCED_USER_INTERFACE, false);
        }
        Self {
            element: Some(element),
            was_enabled,
        }
    }

    /// Whether the app had enhanced UI on when the guard was taken.
    pub fn was_enabled(&self) -> bool {
        self.was_enabled
    }
}

impl Drop for EnhancedUiGuard {
    fn drop(&mut self) {
        if let (true, Some(element)) = (self.was_enabled, self.element) {
            unsafe { set_bool_attribute(element, ENHANCED_USER_INTERFACE, true) };
        }
        // The element is borrowed from the app-element memo, which owns
        // its release — the guard never leaks and never releases.
    }
}

/// # Safety
/// `element` must be a valid, retained `AXUIElementRef`.
unsafe fn set_bool_attribute(element: AXUIElementRef, attribute: &'static str, value: bool) {
    let name = intern(attribute);
    let cf_value = if value {
        kCFBooleanTrue
    } else {
        core_foundation::boolean::kCFBooleanFalse
    };
    let err =
        AXUIElementSetAttributeValue(element, name, cf_value as CFTypeRef);
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
pub unsafe fn is_attribute_settable(element: AXUIElementRef, attribute: &'static str) -> bool {
    let name = intern(attribute);
    let mut settable: std::ffi::c_uchar = 0;
    let err = AXUIElementIsAttributeSettable(element, name, &mut settable);
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
pub unsafe fn string_attribute(element: AXUIElementRef, attribute: &'static str) -> Option<String> {
    let name = intern(attribute);
    let mut value: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(element, name, &mut value);
    if err != kAXErrorSuccess || value.is_null() {
        return None;
    }
    Some(CFString::wrap_under_create_rule(value as CFStringRef).to_string())
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller must ensure the
/// element remains valid for the duration of the call.
pub unsafe fn bool_attribute(element: AXUIElementRef, attribute: &'static str) -> bool {
    let name = intern(attribute);
    let mut value: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(element, name, &mut value);
    if err != kAXErrorSuccess {
        return false;
    }
    let Some(value) = CFRef::from_create(value) else {
        return false;
    };
    CFEqual(value.ptr(), kCFBooleanTrue as CFTypeRef) != 0
}

#[cfg(test)]
mod tests {
    // rects_close is canonical in pengwm-core::layout — tested there.
    // AX writer behavior is exercised through TestAdapter outcomes.
}
