use pengwm_core::command::Command;

use super::codec::{key_name_to_keycode, parse_action, parse_modifiers, split_keybind_str};

pub type ModifierFlags = u64;

pub const MODIFIER_NONE: ModifierFlags = 0;
pub const MODIFIER_CMD: ModifierFlags = 0x0010_0000;
pub const MODIFIER_ALT: ModifierFlags = 0x0008_0000;
pub const MODIFIER_CTRL: ModifierFlags = 0x0004_0000;
pub const MODIFIER_SHIFT: ModifierFlags = 0x0002_0000;

#[derive(Debug, Clone)]
pub struct Keybind {
    pub keycode: u16,
    pub modifiers: ModifierFlags,
    pub action: Command,
}

#[derive(Debug, Clone)]
pub struct KeybindConfig {
    pub bindings: Vec<Keybind>,
}

impl KeybindConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load() -> Self {
        let path = crate::config::config_file_path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => match contents.parse::<toml::Value>() {
                Ok(value) => from_toml_value(&value),
                Err(e) => {
                    log::warn!(
                        "Failed to parse keybinds '{}': {}. Using defaults.",
                        path.display(),
                        e
                    );
                    Self::default()
                }
            },
            Err(_) => {
                log::info!(
                    "No keybinds config at '{}'. Using defaults.",
                    path.display()
                );
                Self::default()
            }
        }
    }
}

pub fn find_keybind(
    keycode: u16,
    modifiers: ModifierFlags,
    config: &KeybindConfig,
) -> Option<Command> {
    for bind in &config.bindings {
        if bind.keycode == keycode && bind.modifiers == modifiers {
            return Some(bind.action.clone());
        }
    }
    None
}

pub fn from_toml_value(value: &toml::Value) -> KeybindConfig {
    let mut bindings = Vec::new();
    let table = match value.as_table() {
        Some(t) => t,
        None => return KeybindConfig { bindings },
    };
    for (key_str, action_val) in table {
        let action_str = match action_val.as_str() {
            Some(s) => s,
            None => continue,
        };
        let action = match parse_action(action_str) {
            Some(a) => a,
            None => continue,
        };
        let (modifier_str, key_name) = split_keybind_str(key_str);
        let modifiers = parse_modifiers(modifier_str);
        let keycode = match key_name_to_keycode(key_name) {
            Some(c) => c,
            None => continue,
        };
        bindings.push(Keybind {
            keycode,
            modifiers,
            action,
        });
    }
    KeybindConfig { bindings }
}

pub fn try_from_toml_value(value: &toml::Value) -> Result<KeybindConfig, String> {
    let mut bindings = Vec::new();
    let table = match value.as_table() {
        Some(t) => t,
        None => return Ok(KeybindConfig { bindings }),
    };
    for (key_str, action_val) in table {
        let action_str = match action_val.as_str() {
            Some(s) => s,
            None => return Err(format!("keybind '{key_str}' value must be a string")),
        };
        let action = parse_action(action_str)
            .ok_or_else(|| format!("unknown action '{action_str}' for keybind '{key_str}'"))?;
        let (modifier_str, key_name) = split_keybind_str(key_str);
        let modifiers = parse_modifiers(modifier_str);
        let keycode = key_name_to_keycode(key_name)
            .ok_or_else(|| format!("unknown key '{key_name}' for keybind '{key_str}'"))?;
        // Reject bindings with no modifiers and no recognized key? 0x00 already handled.
        bindings.push(Keybind {
            keycode,
            modifiers,
            action,
        });
    }
    Ok(KeybindConfig { bindings })
}
