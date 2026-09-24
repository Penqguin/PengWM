use pengwm_core::command::Command;

use super::store::{
    ModifierFlags, MODIFIER_ALT, MODIFIER_CMD, MODIFIER_CTRL, MODIFIER_NONE, MODIFIER_SHIFT,
};

/// The string codec: key names, modifier lists, and action strings.
/// `PrefixKey` parses its chord through here so the two surfaces share one
/// spelling of every key. Testable without the default table.
pub fn key_name_to_keycode(name: &str) -> Option<u16> {
    match name {
        "a" => Some(0x00),
        "b" => Some(0x0B),
        "c" => Some(0x08),
        "d" => Some(0x02),
        "e" => Some(0x0E),
        "f" => Some(0x03),
        "g" => Some(0x05),
        "h" => Some(0x04),
        "i" => Some(0x22),
        "j" => Some(0x26),
        "k" => Some(0x28),
        "l" => Some(0x25),
        "m" => Some(0x2E),
        "n" => Some(0x2D),
        "o" => Some(0x1F),
        "p" => Some(0x23),
        "q" => Some(0x0C),
        "r" => Some(0x0F),
        "s" => Some(0x01),
        "t" => Some(0x11),
        "u" => Some(0x20),
        "v" => Some(0x09),
        "w" => Some(0x0D),
        "x" => Some(0x07),
        "y" => Some(0x10),
        "z" => Some(0x06),
        "0" => Some(0x1D),
        "1" => Some(0x12),
        "2" => Some(0x13),
        "3" => Some(0x14),
        "4" => Some(0x15),
        "5" => Some(0x17),
        "6" => Some(0x16),
        "7" => Some(0x1A),
        "8" => Some(0x1B),
        "9" => Some(0x19),
        "left" => Some(0x7B),
        "right" => Some(0x7C),
        "down" => Some(0x7D),
        "up" => Some(0x7E),
        "," => Some(0x2B),
        "." => Some(0x2F),
        "/" => Some(0x2C),
        "space" => Some(0x31),
        "tab" => Some(0x30),
        "escape" => Some(0x35),
        "return" => Some(0x24),
        "delete" => Some(0x33),
        "home" => Some(0x73),
        "end" => Some(0x77),
        "pageup" => Some(0x74),
        "pagedown" => Some(0x79),
        _ => None,
    }
}

pub fn parse_modifiers(s: &str) -> ModifierFlags {
    if s.is_empty() {
        return MODIFIER_NONE;
    }
    let mut flags = MODIFIER_NONE;
    for part in s.split('-') {
        match part.trim().to_lowercase().as_str() {
            "cmd" | "command" => flags |= MODIFIER_CMD,
            "alt" | "option" => flags |= MODIFIER_ALT,
            "ctrl" | "control" => flags |= MODIFIER_CTRL,
            "shift" => flags |= MODIFIER_SHIFT,
            _ => {}
        }
    }
    flags
}

pub fn parse_action(s: &str) -> Option<Command> {
    Command::parse_action(s)
}

pub(crate) fn split_keybind_str(s: &str) -> (&str, &str) {
    if let Some(last_dash) = s.rfind('-') {
        let mod_part = &s[..last_dash];
        let key_part = &s[last_dash + 1..];
        (mod_part, key_part)
    } else {
        ("", s)
    }
}
