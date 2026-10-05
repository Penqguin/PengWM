use super::*;
use pengwm_core::command::Command;
use pengwm_core::tree::Direction;
use pengwm_core::workspace::LayoutPreset;

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
fn default_has_layout_cycling_and_magnify() {
    let config = KeybindConfig::default();
    assert!(config.bindings.iter().any(|b| b.keycode == 0x11
        && b.modifiers == MODIFIER_ALT
        && matches!(&b.action, Command::CycleLayout)));
    assert!(config.bindings.iter().any(|b| b.keycode == 0x2E
        && b.modifiers == MODIFIER_ALT
        && matches!(&b.action, Command::ToggleMagnify)));
}

#[test]
fn default_has_no_comma_dot_slash_layout_bindings() {
    let config = KeybindConfig::default();
    for keycode in [0x2Bu16, 0x2Fu16, 0x2Cu16] {
        assert!(
            !config
                .bindings
                .iter()
                .any(|b| b.keycode == keycode && b.modifiers == MODIFIER_ALT),
            "keycode {keycode:#X} should be unbound"
        );
    }
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
fn find_keybind_layout_cycle_and_magnify() {
    let config = KeybindConfig::default();
    let cycle = find_keybind(0x11, MODIFIER_ALT, &config);
    assert!(matches!(cycle, Some(Command::CycleLayout)));
    let mag = find_keybind(0x2E, MODIFIER_ALT, &config);
    assert!(matches!(mag, Some(Command::ToggleMagnify)));
    assert!(find_keybind(0x2C, MODIFIER_ALT, &config).is_none());
    assert!(find_keybind(0x2B, MODIFIER_ALT, &config).is_none());
}

#[test]
fn key_name_to_keycode_arrows() {
    assert_eq!(key_name_to_keycode("left"), Some(0x7B));
    assert_eq!(key_name_to_keycode("right"), Some(0x7C));
    assert_eq!(key_name_to_keycode("up"), Some(0x7E));
    assert_eq!(key_name_to_keycode("down"), Some(0x7D));
}

#[test]
fn key_name_to_keycode_letters() {
    assert_eq!(key_name_to_keycode("h"), Some(0x04));
    assert_eq!(key_name_to_keycode("j"), Some(0x26));
    assert_eq!(key_name_to_keycode("k"), Some(0x28));
    assert_eq!(key_name_to_keycode("l"), Some(0x25));
}

#[test]
fn key_name_to_keycode_digits() {
    assert_eq!(key_name_to_keycode("1"), Some(0x12));
    assert_eq!(key_name_to_keycode("9"), Some(0x19));
}

#[test]
fn key_name_to_keycode_punctuation() {
    assert_eq!(key_name_to_keycode(","), Some(0x2B));
    assert_eq!(key_name_to_keycode("/"), Some(0x2C));
}

#[test]
fn key_name_to_keycode_unknown_returns_zero() {
    assert_eq!(key_name_to_keycode("foobar"), None);
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
fn default_has_resize_pane_on_alt_shift_arrows() {
    let config = KeybindConfig::default();
    let mods = MODIFIER_ALT | MODIFIER_SHIFT;
    for (keycode, dir) in [
        (0x7B, Direction::Left),
        (0x7C, Direction::Right),
        (0x7E, Direction::Up),
        (0x7D, Direction::Down),
    ] {
        assert!(
            config.bindings.iter().any(|b| b.keycode == keycode
                && b.modifiers == mods
                && matches!(&b.action, Command::ResizePane { direction: d } if *d == dir)),
            "missing resize-pane binding for keycode {keycode:#X}"
        );
    }
}

#[test]
fn default_has_preset_layouts_on_alt_ctrl_digits() {
    let config = KeybindConfig::default();
    let mods = MODIFIER_ALT | MODIFIER_CTRL;
    for (keycode, preset) in [
        (0x12, LayoutPreset::EvenHorizontal),
        (0x13, LayoutPreset::EvenVertical),
        (0x14, LayoutPreset::MainHorizontal),
        (0x15, LayoutPreset::MainVertical),
        (0x17, LayoutPreset::Tiled),
    ] {
        assert!(
            config.bindings.iter().any(|b| b.keycode == keycode
                && b.modifiers == mods
                && matches!(&b.action, Command::SelectLayout { preset: p } if *p == preset)),
            "missing select-layout binding for keycode {keycode:#X}"
        );
    }
}

#[test]
fn try_from_toml_valid() {
    let toml_str = r#"
cmd-h = "focus-left"
cmd-shift-j = "move-window-down"
cmd-1 = "workspace-1"
"#;
    let value: toml::Value = toml::from_str(toml_str).unwrap();
    let config = try_from_toml_value(&value).unwrap();
    assert_eq!(config.bindings.len(), 3);
}

#[test]
fn try_from_toml_rejects_unknown_action() {
    let toml_str = r#"
cmd-h = "focus-left"
cmd-x = "bogus-action"
"#;
    let value: toml::Value = toml::from_str(toml_str).unwrap();
    let err = try_from_toml_value(&value).unwrap_err();
    assert!(err.contains("bogus-action"), "unexpected error: {err}");
}

#[test]
fn try_from_toml_rejects_invalid_workspace_id() {
    let toml_str = r#"alt-z = "workspace-0""#;
    let value: toml::Value = toml::from_str(toml_str).unwrap();
    let err = try_from_toml_value(&value).unwrap_err();
    assert!(err.contains("workspace-0"), "unexpected error: {err}");
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
