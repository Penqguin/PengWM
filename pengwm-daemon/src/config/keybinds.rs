//! Keybinding parsing and representation.
//!
//! Keybinds are defined in config.toml like:
//!
//!   [keybinds]
//!   cmd-left  = "focus-left"
//!   cmd-shift-left = "swap-left"
//!   cmd-1     = "workspace-1"
//!   alt-tab   = "next-workspace"

use serde::{Serialize, Deserialize};

// ---------------------------------------------------------------------------
// Keybind representation
// ---------------------------------------------------------------------------

/// A single keybind mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keybind {
    // pub keycode: u16,   // CGKeyCode
    // pub modifiers: ModifierFlags,
    // pub action: String, // matches a DaemonCommand variant name
}

/// Bitmask of modifier keys.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModifierFlags {
    // pub cmd: bool,
    // pub alt: bool,
    // pub ctrl: bool,
    // pub shift: bool,
    // pub fn: bool,
}

/// Parsed keybind configuration, indexed by (modifiers, keycode) for O(1) lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindConfig {
    //  bindings: HashMap<(u16, u64), String>,   // (keycode, modifierFlags) -> action name
}

impl Default for KeybindConfig {
    fn default() -> Self {
        //  set sensible defaults:
        //    cmd-h         -> "focus-left"
        //    cmd-l         -> "focus-right"
        //    cmd-k         -> "focus-up"
        //    cmd-j         -> "focus-down"
        //    cmd-shift-h   -> "swap-left", etc...
        //    cmd-1..9      -> "switch-workspace-1..9"
        todo!()
    }
}

impl KeybindConfig {
    /// Parse a list of TOML keybind entries into the config.
    pub fn from_toml(keybinds: &toml::Value) -> Self {
        //  for each (key_str, action_str) in the table:
        //     parse key_str into (keycode, modifiers)
        //     parse action_str into a command name
        //     insert into self.bindings
        todo!()
    }
}

// ---------------------------------------------------------------------------
// Keycode utilities
// ---------------------------------------------------------------------------

/// Map a key name (e.g. "left", "h", "1", "space") to a CGKeyCode.
fn key_name_to_keycode(name: &str) -> u16 {
    //  lookup table for common key names
    todo!()
}

/// Parse a modifier string like "cmd-shift" into ModifierFlags.
fn parse_modifiers(s: &str) -> ModifierFlags {
    //  split on '-', check for each modifier
    todo!()
}
