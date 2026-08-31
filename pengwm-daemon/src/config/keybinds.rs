use pengwm_core::command::{Command, LayoutMode};
use pengwm_core::tree::Direction;

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

impl Default for KeybindConfig {
    fn default() -> Self {
        let bindings = vec![
            // Focus movement: alt-h/j/k/l (vim-style)
            Keybind {
                keycode: 0x04,
                modifiers: MODIFIER_ALT,
                action: Command::Focus {
                    direction: Direction::Left,
                },
            },
            Keybind {
                keycode: 0x26,
                modifiers: MODIFIER_ALT,
                action: Command::Focus {
                    direction: Direction::Down,
                },
            },
            Keybind {
                keycode: 0x28,
                modifiers: MODIFIER_ALT,
                action: Command::Focus {
                    direction: Direction::Up,
                },
            },
            Keybind {
                keycode: 0x25,
                modifiers: MODIFIER_ALT,
                action: Command::Focus {
                    direction: Direction::Right,
                },
            },
            // Arrow keys as alternative
            Keybind {
                keycode: 0x7B,
                modifiers: MODIFIER_ALT,
                action: Command::Focus {
                    direction: Direction::Left,
                },
            },
            Keybind {
                keycode: 0x7D,
                modifiers: MODIFIER_ALT,
                action: Command::Focus {
                    direction: Direction::Down,
                },
            },
            Keybind {
                keycode: 0x7E,
                modifiers: MODIFIER_ALT,
                action: Command::Focus {
                    direction: Direction::Up,
                },
            },
            Keybind {
                keycode: 0x7C,
                modifiers: MODIFIER_ALT,
                action: Command::Focus {
                    direction: Direction::Right,
                },
            },
            // Move window: alt-shift-h/j/k/l (swap places and resize)
            Keybind {
                keycode: 0x04,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindow {
                    direction: Direction::Left,
                },
            },
            Keybind {
                keycode: 0x26,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindow {
                    direction: Direction::Down,
                },
            },
            Keybind {
                keycode: 0x28,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindow {
                    direction: Direction::Up,
                },
            },
            Keybind {
                keycode: 0x25,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindow {
                    direction: Direction::Right,
                },
            },
            // Workspace switching: alt-1..9
            Keybind {
                keycode: 0x12,
                modifiers: MODIFIER_ALT,
                action: Command::Workspace { id: 1 },
            },
            Keybind {
                keycode: 0x13,
                modifiers: MODIFIER_ALT,
                action: Command::Workspace { id: 2 },
            },
            Keybind {
                keycode: 0x14,
                modifiers: MODIFIER_ALT,
                action: Command::Workspace { id: 3 },
            },
            Keybind {
                keycode: 0x15,
                modifiers: MODIFIER_ALT,
                action: Command::Workspace { id: 4 },
            },
            Keybind {
                keycode: 0x17,
                modifiers: MODIFIER_ALT,
                action: Command::Workspace { id: 5 },
            },
            Keybind {
                keycode: 0x16,
                modifiers: MODIFIER_ALT,
                action: Command::Workspace { id: 6 },
            },
            Keybind {
                keycode: 0x1A,
                modifiers: MODIFIER_ALT,
                action: Command::Workspace { id: 7 },
            },
            Keybind {
                keycode: 0x1B,
                modifiers: MODIFIER_ALT,
                action: Command::Workspace { id: 8 },
            },
            Keybind {
                keycode: 0x19,
                modifiers: MODIFIER_ALT,
                action: Command::Workspace { id: 9 },
            },
            // Move window to workspace: alt-shift-1..9
            Keybind {
                keycode: 0x12,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindowToWorkspace { id: 1 },
            },
            Keybind {
                keycode: 0x13,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindowToWorkspace { id: 2 },
            },
            Keybind {
                keycode: 0x14,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindowToWorkspace { id: 3 },
            },
            Keybind {
                keycode: 0x15,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindowToWorkspace { id: 4 },
            },
            Keybind {
                keycode: 0x17,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindowToWorkspace { id: 5 },
            },
            Keybind {
                keycode: 0x16,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindowToWorkspace { id: 6 },
            },
            Keybind {
                keycode: 0x1A,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindowToWorkspace { id: 7 },
            },
            Keybind {
                keycode: 0x1B,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindowToWorkspace { id: 8 },
            },
            Keybind {
                keycode: 0x19,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::MoveWindowToWorkspace { id: 9 },
            },
            // Layout: alt-/ tile, alt-, accordion (monocle)
            Keybind {
                keycode: 0x2C,
                modifiers: MODIFIER_ALT,
                action: Command::SetLayout {
                    mode: LayoutMode::Tile,
                },
            },
            Keybind {
                keycode: 0x2B,
                modifiers: MODIFIER_ALT,
                action: Command::SetLayout {
                    mode: LayoutMode::Accordion,
                },
            },
            // Reload config: cmd-shift-r
            Keybind {
                keycode: 0x0F,
                modifiers: MODIFIER_CMD | MODIFIER_SHIFT,
                action: Command::ReloadConfig,
            },
            // Toggle bar: alt-b
            Keybind {
                keycode: 0x0B,
                modifiers: MODIFIER_ALT,
                action: Command::ToggleBar,
            },
        ];
        KeybindConfig { bindings }
    }
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

pub fn key_name_to_keycode(name: &str) -> u16 {
    match name {
        "a" => 0x00,
        "b" => 0x0B,
        "c" => 0x08,
        "d" => 0x02,
        "e" => 0x0E,
        "f" => 0x03,
        "g" => 0x05,
        "h" => 0x04,
        "i" => 0x22,
        "j" => 0x26,
        "k" => 0x28,
        "l" => 0x25,
        "m" => 0x2E,
        "n" => 0x2D,
        "o" => 0x1F,
        "p" => 0x23,
        "q" => 0x0C,
        "r" => 0x0F,
        "s" => 0x01,
        "t" => 0x11,
        "u" => 0x20,
        "v" => 0x09,
        "w" => 0x0D,
        "x" => 0x07,
        "y" => 0x10,
        "z" => 0x06,
        "0" => 0x1D,
        "1" => 0x12,
        "2" => 0x13,
        "3" => 0x14,
        "4" => 0x15,
        "5" => 0x17,
        "6" => 0x16,
        "7" => 0x1A,
        "8" => 0x1B,
        "9" => 0x19,
        "left" => 0x7B,
        "right" => 0x7C,
        "down" => 0x7D,
        "up" => 0x7E,
        "," => 0x2B,
        "/" => 0x2C,
        "space" => 0x31,
        "tab" => 0x30,
        "escape" => 0x35,
        "return" => 0x24,
        "delete" => 0x33,
        "home" => 0x73,
        "end" => 0x77,
        "pageup" => 0x74,
        "pagedown" => 0x79,
        _ => 0x00,
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
        let keycode = key_name_to_keycode(key_name);
        bindings.push(Keybind {
            keycode,
            modifiers,
            action,
        });
    }
    KeybindConfig { bindings }
}

fn split_keybind_str(s: &str) -> (&str, &str) {
    if let Some(last_dash) = s.rfind('-') {
        let mod_part = &s[..last_dash];
        let key_part = &s[last_dash + 1..];
        (mod_part, key_part)
    } else {
        ("", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_vim_navigation() {
        let config = KeybindConfig::default();
        assert!(config
            .bindings
            .iter()
            .any(|b| b.keycode == 0x04 && b.modifiers == MODIFIER_ALT));
        assert!(config
            .bindings
            .iter()
            .any(|b| b.keycode == 0x26 && b.modifiers == MODIFIER_ALT));
        assert!(config
            .bindings
            .iter()
            .any(|b| b.keycode == 0x28 && b.modifiers == MODIFIER_ALT));
        assert!(config
            .bindings
            .iter()
            .any(|b| b.keycode == 0x25 && b.modifiers == MODIFIER_ALT));
    }

    #[test]
    fn default_has_swap_modifiers() {
        let config = KeybindConfig::default();
        let mods = MODIFIER_ALT | MODIFIER_SHIFT;
        assert!(config
            .bindings
            .iter()
            .any(|b| b.keycode == 0x04 && b.modifiers == mods));
        assert!(config
            .bindings
            .iter()
            .any(|b| b.keycode == 0x26 && b.modifiers == mods));
    }

    #[test]
    fn default_has_workspace_switching() {
        let config = KeybindConfig::default();
        for i in 1..=9 {
            assert!(config
                .bindings
                .iter()
                .any(|b| matches!(&b.action, Command::Workspace { id } if *id == i)));
        }
    }

    #[test]
    fn default_has_move_to_workspace() {
        let config = KeybindConfig::default();
        let mods = MODIFIER_ALT | MODIFIER_SHIFT;
        assert!(config
            .bindings
            .iter()
            .any(|b| b.keycode == 0x12 && b.modifiers == mods));
    }

    #[test]
    fn default_has_layout_switching() {
        let config = KeybindConfig::default();
        assert!(config.bindings.iter().any(|b| matches!(
            &b.action,
            Command::SetLayout {
                mode: LayoutMode::Tile
            }
        )));
        assert!(config.bindings.iter().any(|b| matches!(
            &b.action,
            Command::SetLayout {
                mode: LayoutMode::Accordion
            }
        )));
    }

    #[test]
    fn default_layout_bindings_on_slash_and_comma() {
        let config = KeybindConfig::default();
        assert!(config.bindings.iter().any(|b| b.keycode == 0x2C
            && b.modifiers == MODIFIER_ALT
            && matches!(
                &b.action,
                Command::SetLayout {
                    mode: LayoutMode::Tile
                }
            )));
        assert!(config.bindings.iter().any(|b| b.keycode == 0x2B
            && b.modifiers == MODIFIER_ALT
            && matches!(
                &b.action,
                Command::SetLayout {
                    mode: LayoutMode::Accordion
                }
            )));
    }

    #[test]
    fn find_keybind_matches() {
        let config = KeybindConfig::default();
        let result = find_keybind(0x04, MODIFIER_ALT, &config);
        assert!(matches!(
            result,
            Some(Command::Focus {
                direction: Direction::Left
            })
        ));
    }

    #[test]
    fn find_keybind_no_match() {
        let config = KeybindConfig::default();
        let result = find_keybind(0xFF, 0, &config);
        assert!(result.is_none());
    }

    #[test]
    fn find_keybind_wrong_modifiers() {
        let config = KeybindConfig::default();
        let result = find_keybind(0x04, MODIFIER_CMD, &config);
        assert!(result.is_none());
    }

    #[test]
    fn find_keybind_layout_switch() {
        let config = KeybindConfig::default();
        let tile = find_keybind(0x2C, MODIFIER_ALT, &config);
        assert!(matches!(
            tile,
            Some(Command::SetLayout {
                mode: LayoutMode::Tile
            })
        ));
        let accordion = find_keybind(0x2B, MODIFIER_ALT, &config);
        assert!(matches!(
            accordion,
            Some(Command::SetLayout {
                mode: LayoutMode::Accordion
            })
        ));
    }

    #[test]
    fn key_name_to_keycode_arrows() {
        assert_eq!(key_name_to_keycode("left"), 0x7B);
        assert_eq!(key_name_to_keycode("right"), 0x7C);
        assert_eq!(key_name_to_keycode("up"), 0x7E);
        assert_eq!(key_name_to_keycode("down"), 0x7D);
    }

    #[test]
    fn key_name_to_keycode_letters() {
        assert_eq!(key_name_to_keycode("h"), 0x04);
        assert_eq!(key_name_to_keycode("j"), 0x26);
        assert_eq!(key_name_to_keycode("k"), 0x28);
        assert_eq!(key_name_to_keycode("l"), 0x25);
    }

    #[test]
    fn key_name_to_keycode_digits() {
        assert_eq!(key_name_to_keycode("1"), 0x12);
        assert_eq!(key_name_to_keycode("9"), 0x19);
    }

    #[test]
    fn key_name_to_keycode_punctuation() {
        assert_eq!(key_name_to_keycode(","), 0x2B);
        assert_eq!(key_name_to_keycode("/"), 0x2C);
    }

    #[test]
    fn key_name_to_keycode_unknown_returns_zero() {
        assert_eq!(key_name_to_keycode("foobar"), 0x00);
    }

    #[test]
    fn parse_modifiers_cmd() {
        assert_eq!(parse_modifiers("cmd"), MODIFIER_CMD);
    }

    #[test]
    fn parse_modifiers_cmd_shift() {
        assert_eq!(parse_modifiers("cmd-shift"), MODIFIER_CMD | MODIFIER_SHIFT);
    }

    #[test]
    fn parse_modifiers_all() {
        let all = MODIFIER_CMD | MODIFIER_ALT | MODIFIER_CTRL | MODIFIER_SHIFT;
        assert_eq!(parse_modifiers("cmd-alt-ctrl-shift"), all);
    }

    #[test]
    fn parse_modifiers_empty() {
        assert_eq!(parse_modifiers(""), 0);
    }

    #[test]
    fn parse_modifiers_case_insensitive() {
        assert_eq!(parse_modifiers("CMD-SHIFT"), MODIFIER_CMD | MODIFIER_SHIFT);
    }

    #[test]
    fn parse_modifiers_invalid_part_ignored() {
        assert_eq!(
            parse_modifiers("cmd-foo-shift"),
            MODIFIER_CMD | MODIFIER_SHIFT
        );
    }

    #[test]
    fn parse_modifiers_full_names() {
        assert_eq!(parse_modifiers("command"), MODIFIER_CMD);
        assert_eq!(parse_modifiers("option"), MODIFIER_ALT);
        assert_eq!(parse_modifiers("control"), MODIFIER_CTRL);
    }

    #[test]
    fn default_has_toggle_bar() {
        let config = KeybindConfig::default();
        assert!(config.bindings.iter().any(|b| {
            b.keycode == 0x0B
                && b.modifiers == MODIFIER_ALT
                && matches!(b.action, Command::ToggleBar)
        }));
    }

    #[test]
    fn from_toml_valid() {
        let toml_str = r#"
cmd-h = "focus-left"
cmd-shift-j = "move-window-down"
cmd-1 = "workspace-1"
"#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let config = from_toml_value(&value);
        assert_eq!(config.bindings.len(), 3);
    }

    #[test]
    fn from_toml_invalid_ignored() {
        let toml_str = r#"
cmd-h = "focus-left"
cmd-x = "bogus-action"
alt-z = "workspace-0"
"#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let config = from_toml_value(&value);
        assert_eq!(config.bindings.len(), 1);
    }

    #[test]
    fn split_keybind_str_basic() {
        let (mods, key) = split_keybind_str("cmd-shift-h");
        assert_eq!(mods, "cmd-shift");
        assert_eq!(key, "h");
    }

    #[test]
    fn split_keybind_str_no_modifier() {
        let (mods, key) = split_keybind_str("space");
        assert_eq!(mods, "");
        assert_eq!(key, "space");
    }
}
