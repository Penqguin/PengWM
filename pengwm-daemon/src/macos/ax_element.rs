use std::ffi::c_void;
use std::ptr;

use accessibility_sys::*;
use core_foundation::base::{CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::boolean::kCFBooleanTrue;
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::array::{CFArrayRef, CFArrayGetCount, CFArrayGetValueAtIndex};

use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;

pub fn is_process_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub fn is_process_trusted_with_prompt() -> bool {
    unsafe {
        let key = CFString::new("AXTrustedCheckOptionPrompt");
        let val = CFNumber::from(1i32);
        let dict = CFDictionary::from_CFType_pairs(
            &[(key, val)],
        );
        AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef())
    }
}

/// Show the system Accessibility permission prompt dialog.
/// macOS will open System Settings so the user can grant access.
pub fn request_trusted_access() {
    let _ = is_process_trusted_with_prompt();
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
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller must ensure the
/// element is valid and that the Accessibility API can be called safely.
pub unsafe fn set_window_rect(element: AXUIElementRef, rect: Rect) -> anyhow::Result<()> {
    let pos_name = CFString::new(kAXPositionAttribute);
    let size_name = CFString::new(kAXSizeAttribute);

    let mut point = CGPoint { x: rect.x, y: rect.y };
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
        anyhow::bail!("AXUIElementSetAttributeValue position error: {}", error_string(err));
    }

    let mut size = CGSize { width: rect.width, height: rect.height };
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
        anyhow::bail!("AXUIElementSetAttributeValue size error: {}", error_string(err));
    }

    Ok(())
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller must ensure the
/// element remains valid for the duration of the call.
pub unsafe fn get_window_rect(element: AXUIElementRef) -> Option<Rect> {
    let pos_name = CFString::new(kAXPositionAttribute);
    let size_name = CFString::new(kAXSizeAttribute);

    let mut pos_val: CFTypeRef = ptr::null();
    let err_pos = AXUIElementCopyAttributeValue(element, pos_name.as_concrete_TypeRef(), &mut pos_val);
    if err_pos != kAXErrorSuccess || pos_val.is_null() {
        return None;
    }

    let mut size_val: CFTypeRef = ptr::null();
    let err_size = AXUIElementCopyAttributeValue(element, size_name.as_concrete_TypeRef(), &mut size_val);
    if err_size != kAXErrorSuccess || size_val.is_null() {
        CFRelease(pos_val);
        return None;
    }

    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let mut size = CGSize { width: 0.0, height: 0.0 };
    AXValueGetValue(pos_val as AXValueRef, kAXValueTypeCGPoint, &mut point as *mut _ as *mut c_void);
    AXValueGetValue(size_val as AXValueRef, kAXValueTypeCGSize, &mut size as *mut _ as *mut c_void);

    CFRelease(pos_val);
    CFRelease(size_val);

    Some(Rect { x: point.x, y: point.y, width: size.width, height: size.height })
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller must ensure the
/// element is valid for the duration of the call.
pub unsafe fn focus_window(element: AXUIElementRef) {
    let raise_name = CFString::new(kAXRaiseAction);
    AXUIElementPerformAction(element, raise_name.as_concrete_TypeRef());

    let focused_name = CFString::new(kAXFocusedAttribute);
    AXUIElementSetAttributeValue(
        element,
        focused_name.as_concrete_TypeRef(),
        kCFBooleanTrue as CFTypeRef,
    );
}

/// # Safety
///
/// `element` must be a valid, retained `AXUIElementRef`. The caller must ensure the
/// element is valid and that the Accessibility API can be called safely.
pub unsafe fn is_manageable(element: AXUIElementRef) -> bool {
    let role_name = CFString::new(kAXRoleAttribute);
    let mut role_val: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(element, role_name.as_concrete_TypeRef(), &mut role_val);
    if err != kAXErrorSuccess || role_val.is_null() {
        return false;
    }
    let role_str = CFString::wrap_under_create_rule(role_val as CFStringRef);
    if role_str != kAXWindowRole {
        return false;
    }

    let subrole_name = CFString::new(kAXSubroleAttribute);
    let mut subrole_val: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(element, subrole_name.as_concrete_TypeRef(), &mut subrole_val);
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
    let app = AXUIElementCreateApplication(pid);
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
    let window_id = value as u64;
    CFRelease(value);
    Some(window_id)
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
    let app = AXUIElementCreateApplication(pid);
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

/// # Safety
///
/// The caller must ensure that `pid` references a valid running process and that
/// the Accessibility API is called from a trusted process with the necessary permissions.
pub unsafe fn find_element(pid: i32, window_id: WindowId) -> Option<AXUIElementRef> {
    let windows = windows_for_pid(pid);
    for (elem, wid) in &windows {
        if *wid == window_id {
            return Some(*elem);
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_creation() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(r.x, 10.0);
        assert_eq!(r.y, 20.0);
        assert_eq!(r.width, 100.0);
        assert_eq!(r.height, 50.0);
    }

    #[test]
    fn window_id_type_is_compatible() {
        let id: WindowId = 42;
        let _cg_id: u32 = id as u32;
        assert_eq!(_cg_id, 42u32);
    }

    #[test]
    fn axui_element_ref_is_pointer_sized() {
        assert_eq!(std::mem::size_of::<AXUIElementRef>(), std::mem::size_of::<*mut c_void>());
    }

    #[test]
    fn axtype_ref_is_pointer_sized() {
        assert_eq!(std::mem::size_of::<AXValueRef>(), std::mem::size_of::<*mut c_void>());
    }

    #[test]
    fn error_constants_are_negative() {
        const { assert!(kAXErrorSuccess >= 0); }
        const { assert!(kAXErrorFailure < 0); }
        const { assert!(kAXErrorCannotComplete < 0); }
        const { assert!(kAXErrorAttributeUnsupported < 0); }
    }

    #[test]
    fn role_constants_match_expected_strings() {
        assert_eq!(kAXWindowRole, "AXWindow");
        assert_eq!(kAXStandardWindowSubrole, "AXStandardWindow");
        assert_eq!(kAXApplicationRole, "AXApplication");
    }

    #[test]
    fn attribute_constants_match() {
        assert_eq!(kAXPositionAttribute, "AXPosition");
        assert_eq!(kAXSizeAttribute, "AXSize");
        assert_eq!(kAXRoleAttribute, "AXRole");
        assert_eq!(kAXSubroleAttribute, "AXSubrole");
        assert_eq!(kAXFocusedAttribute, "AXFocused");
        assert_eq!(kAXWindowAttribute, "AXWindow");
        assert_eq!(kAXWindowsAttribute, "AXWindows");
        assert_eq!(kAXFocusedWindowAttribute, "AXFocusedWindow");
    }

    #[test]
    fn action_constants_match() {
        assert_eq!(kAXRaiseAction, "AXRaise");
    }

    #[test]
    fn axvalue_types_match() {
        assert_eq!(kAXValueTypeCGPoint, 1);
        assert_eq!(kAXValueTypeCGSize, 2);
        assert_eq!(kAXValueTypeCGRect, 3);
    }
}
