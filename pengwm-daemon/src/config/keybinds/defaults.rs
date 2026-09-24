use pengwm_core::command::{Command, LayoutMode};
use pengwm_core::tree::Direction;
use pengwm_core::workspace::LayoutPreset;

use super::store::{
    Keybind, KeybindConfig, MODIFIER_ALT, MODIFIER_CMD, MODIFIER_CTRL, MODIFIER_SHIFT,
};

/// The default binding table, isolated so adding a `Command` variant or
/// preset touches only this module — never the codec or the store.
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
            // Layout: alt-/ tile, alt-, accordion (monocle), alt-. tile (alt-,/alt-. pair), alt-t toggle
            Keybind {
                keycode: 0x2C,
                modifiers: MODIFIER_ALT,
                action: Command::SetLayout {
                    mode: LayoutMode::Tile,
                },
            },
            Keybind {
                keycode: 0x2F,
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
            Keybind {
                keycode: 0x11,
                modifiers: MODIFIER_ALT,
                action: Command::ToggleLayout,
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
            // Display focus: alt-ctrl arrows (move focus between monitors)
            Keybind {
                keycode: 0x7B,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL,
                action: Command::FocusDisplay {
                    direction: Direction::Left,
                },
            },
            Keybind {
                keycode: 0x7C,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL,
                action: Command::FocusDisplay {
                    direction: Direction::Right,
                },
            },
            Keybind {
                keycode: 0x7E,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL,
                action: Command::FocusDisplay {
                    direction: Direction::Up,
                },
            },
            Keybind {
                keycode: 0x7D,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL,
                action: Command::FocusDisplay {
                    direction: Direction::Down,
                },
            },
            // Move window to display: alt-ctrl-shift arrows
            Keybind {
                keycode: 0x7B,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL | MODIFIER_SHIFT,
                action: Command::MoveWindowToDisplay {
                    direction: Direction::Left,
                },
            },
            Keybind {
                keycode: 0x7C,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL | MODIFIER_SHIFT,
                action: Command::MoveWindowToDisplay {
                    direction: Direction::Right,
                },
            },
            Keybind {
                keycode: 0x7E,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL | MODIFIER_SHIFT,
                action: Command::MoveWindowToDisplay {
                    direction: Direction::Up,
                },
            },
            Keybind {
                keycode: 0x7D,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL | MODIFIER_SHIFT,
                action: Command::MoveWindowToDisplay {
                    direction: Direction::Down,
                },
            },
            // Resize pane: alt-shift-arrows (push divider, 5% steps)
            Keybind {
                keycode: 0x7B,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::ResizePane {
                    direction: Direction::Left,
                },
            },
            Keybind {
                keycode: 0x7C,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::ResizePane {
                    direction: Direction::Right,
                },
            },
            Keybind {
                keycode: 0x7E,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::ResizePane {
                    direction: Direction::Up,
                },
            },
            Keybind {
                keycode: 0x7D,
                modifiers: MODIFIER_ALT | MODIFIER_SHIFT,
                action: Command::ResizePane {
                    direction: Direction::Down,
                },
            },
            // Preset layouts: alt-ctrl-1..5 (also reachable as prefix follow-ups)
            Keybind {
                keycode: 0x12,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL,
                action: Command::SelectLayout {
                    preset: LayoutPreset::EvenHorizontal,
                },
            },
            Keybind {
                keycode: 0x13,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL,
                action: Command::SelectLayout {
                    preset: LayoutPreset::EvenVertical,
                },
            },
            Keybind {
                keycode: 0x14,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL,
                action: Command::SelectLayout {
                    preset: LayoutPreset::MainHorizontal,
                },
            },
            Keybind {
                keycode: 0x15,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL,
                action: Command::SelectLayout {
                    preset: LayoutPreset::MainVertical,
                },
            },
            Keybind {
                keycode: 0x17,
                modifiers: MODIFIER_ALT | MODIFIER_CTRL,
                action: Command::SelectLayout {
                    preset: LayoutPreset::Tiled,
                },
            },
        ];
        KeybindConfig { bindings }
    }
}
