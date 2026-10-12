//! RAII owner of a +1 CoreFoundation reference.
//!
//! Every CF object this daemon touches is created by a Create/Copy call
//! (+1 owned) or arrives under a Get rule (borrowed). Until this wrapper
//! existed, balance was a convention: each call site released its ref by
//! hand, on every error path, and the element cache released on overwrite
//! via a hand-rolled `insert`-returns-old dance.
//!
//! `CFRef<P>` turns the convention into a type: `Drop` releases, `Clone`
//! retains, map insert/replace/free release by construction. The deletion
//! test says it earns its keep — deleting it scatters ~30 hand-written
//! `CFRelease`s back across the FFI layer.
//!
//! Null is modeled as `Option<CFRef<P>>`: `from_create`/`from_get` return
//! `None` without releasing (a null has no +1 to drop), so null-checks and
//! ownership travel together and never diverge.

use core_foundation::array::CFArrayRef;
use core_foundation::base::{CFRelease, CFRetain, CFTypeRef};

use accessibility_sys::{AXObserverRef, AXUIElementRef, AXValueRef};

/// A raw CF pointer type this wrapper can own. One impl per pointer kind
/// the daemon holds; `release`/`retain` go through the `CFTypeRef` cast
/// every raw call site already did.
pub trait CFPtr: Copy {
    fn as_cf_type(self) -> CFTypeRef;
    fn from_null() -> Self;
    fn is_null(self) -> bool;
}

macro_rules! impl_cf_ptr {
    ($ty:ty) => {
        impl CFPtr for $ty {
            fn as_cf_type(self) -> CFTypeRef {
                self as CFTypeRef
            }
            fn from_null() -> Self {
                std::ptr::null_mut()
            }
            fn is_null(self) -> bool {
                self.is_null()
            }
        }
    };
}

impl_cf_ptr!(AXUIElementRef);
impl_cf_ptr!(AXObserverRef);
impl_cf_ptr!(AXValueRef);
impl_cf_ptr!(CFArrayRef);
impl_cf_ptr!(core_foundation::runloop::CFRunLoopSourceRef);
impl_cf_ptr!(core_foundation::mach_port::CFMachPortRef);

/// Generic CF type (the `CFTypeRef` a Copy/Copy ATTRIBUTE call returns
/// before its concrete kind is known). Const pointers need their own
/// impl: `from_null` is `null()`, not `null_mut()`.
impl CFPtr for CFTypeRef {
    fn as_cf_type(self) -> CFTypeRef {
        self
    }
    fn from_null() -> Self {
        std::ptr::null()
    }
    fn is_null(self) -> bool {
        self.is_null()
    }
}

/// Owns one +1 reference to a CoreFoundation object. `Drop` releases it;
/// `Clone` takes a second reference (retain). Hand `&*`-style `cf.ptr()`
/// to C APIs that only borrow for the call's duration.
pub struct CFRef<P: CFPtr> {
    ptr: P,
}

impl<P: CFPtr> CFRef<P> {
    /// Take ownership of a +1 the caller just created (Create/Copy/Retain
    /// result). `None` for null — a null has no reference to own, and the
    /// caller's null-check and the ownership handoff are the same act.
    ///
    /// # Safety
    /// `ptr` must be a valid CF object the caller owns a +1 reference to,
    /// or null.
    pub unsafe fn from_create(ptr: P) -> Option<Self> {
        if ptr.is_null() {
            return None;
        }
        Some(Self { ptr })
    }

    /// Take ownership by retaining a reference the caller only borrowed
    /// (Get-rule results, AX notification payloads). `None` for null.
    ///
    /// # Safety
    /// `ptr` must be a valid CF object or null; the call retains it.
    pub unsafe fn from_get(ptr: P) -> Option<Self> {
        if ptr.is_null() {
            return None;
        }
        CFRetain(ptr.as_cf_type());
        Some(Self { ptr })
    }

    /// The raw pointer, valid while `self` lives (and for any borrowed
    /// call that doesn't outlive it). Never null.
    pub fn ptr(&self) -> P {
        self.ptr
    }

    /// True when two refs point at the same CF object (`CFEqual` by
    /// pointer is the identity AX elements are keyed by in caches).
    pub fn ptr_eq(&self, other: &Self) -> bool {
        self.ptr.as_cf_type() == other.ptr.as_cf_type()
    }

    /// Give up ownership without releasing — the object now belongs to
    /// whatever consumes the raw pointer. Deliberate-leak escape hatch:
    /// every use should come with a comment saying who releases instead.
    pub fn into_raw(self) -> P {
        let ptr = self.ptr;
        std::mem::forget(self);
        ptr
    }
}

impl<P: CFPtr> Clone for CFRef<P> {
    fn clone(&self) -> Self {
        unsafe { CFRetain(self.ptr.as_cf_type()) };
        Self { ptr: self.ptr }
    }
}

impl<P: CFPtr> Drop for CFRef<P> {
    fn drop(&mut self) {
        unsafe { CFRelease(self.ptr.as_cf_type()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A stand-in pointer type for tests: CFRef's logic over P is pointer
    // plumbing, but the real types are opaque C objects with side effects.
    // Asserting the plumbing needs a fake — this one does nothing.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct FakePtr(*mut u8);
    impl CFPtr for FakePtr {
        fn as_cf_type(self) -> CFTypeRef {
            self.0 as CFTypeRef
        }
        fn from_null() -> Self {
            FakePtr(std::ptr::null_mut())
        }
        fn is_null(self) -> bool {
            self.0.is_null()
        }
    }

    #[test]
    fn null_create_yields_none() {
        let r: Option<CFRef<FakePtr>> = unsafe { CFRef::from_create(FakePtr::from_null()) };
        assert!(r.is_none());
        let r: Option<CFRef<FakePtr>> = unsafe { CFRef::from_get(FakePtr::from_null()) };
        assert!(r.is_none());
    }

    #[test]
    fn create_owns_into_raw_escapes_the_release() {
        let fake = FakePtr(0x1234 as *mut u8);
        let r: CFRef<FakePtr> = unsafe { CFRef::from_create(fake) }.unwrap();
        assert_eq!(r.ptr(), fake);
        // Ownership escapes without a release — the caller took the +1 back.
        assert_eq!(r.into_raw(), fake);
    }
}
