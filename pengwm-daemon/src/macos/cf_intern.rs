//! Interned CFString constants for the FFI layer.
//!
//! Every AX attribute name and CG dictionary key handed to Core
//! Foundation is a compile-time constant, and creating a fresh `CFString`
//! per call (alloc + UTF-16 conversion + retain/release) cost the hot
//! thread dozens of heap churn cycles per second for the same handful of
//! strings. One table turns each into a pointer, built on first use.
//!
//! Bounded by construction: the key type is `&'static str`, so only
//! compile-time constants can ever enter the map.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};

/// An interned `CFStringRef` shared across every FFI call. The wrapper
/// exists only to make the raw pointer storable in the process-global
/// table: CF string objects are immutable and thread-safe, and the
/// interned entries never move.
struct Interned(CFStringRef);
unsafe impl Send for Interned {}
unsafe impl Sync for Interned {}

static TABLE: LazyLock<Mutex<HashMap<&'static str, Interned>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The `CFStringRef` for a string constant, interned for the process
/// lifetime. The entry leaks the +1 reference by design — the object's
/// lifetime matches the table's — for a string every future call reuses.
pub fn intern(name: &'static str) -> CFStringRef {
    let mut table = TABLE.lock().unwrap();
    if let Some(entry) = table.get(name) {
        return entry.0;
    }
    let s = CFString::new(name);
    let raw = s.as_concrete_TypeRef();
    std::mem::forget(s);
    table.insert(name, Interned(raw));
    raw
}
