//! HUD tests (Spec 0001): explicit-only triggers, per-monitor card
//! contents, the prefix pin, and expiry through the tick.

use std::time::{Duration, Instant};

use super::common::setup_with_handle;
use crate::config::keybinds::MODIFIER_ALT;
use crate::prefix::{PrefixConfig, PrefixKey};
use pengwm_core::command::Command;

/// An explicit switch announces per monitor — left-to-right, the
/// focused monitor accented. Workspace 2 already lives on monitor 2,
/// so the switch is the focus-rule move: monitor 1 keeps workspace 1,
/// monitor 2 is accented with workspace 2.
#[test]
fn workspace_switch_flashes_per_monitor() {
    let (mut sm, handle) = setup_with_handle(2);
    sm.on_command(Command::Workspace { id: 2 }, None);

    let flashes = handle.hud_flashes();
    assert!(!flashes.is_empty(), "explicit switch flashes the HUD");
    let cards = flashes.last().unwrap();
    assert_eq!(cards.len(), 2, "one card per monitor");
    assert_eq!(cards[0].display_id, 1);
    assert_eq!(cards[0].number, 1, "left monitor keeps workspace 1");
    assert_eq!(cards[0].name, "Development");
    assert!(!cards[0].focused);
    assert_eq!(cards[1].display_id, 2);
    assert_eq!(cards[1].number, 2, "right monitor shows workspace 2");
    assert_eq!(cards[1].name, "Browsing");
    assert!(cards[1].focused, "focus moved to workspace 2's monitor");
}

/// Background work — the tick sweeps with no explicit command — never
/// flashes. The HUD's own sale was silence during typing.
#[test]
fn background_tick_does_not_flash() {
    let (mut sm, handle) = setup_with_handle(2);
    sm.on_tick();
    assert!(handle.hud_flashes().is_empty(), "no explicit command, no flash");
    assert_eq!(handle.hud_hides(), 0, "nothing shown, nothing hidden");
}

/// `hud-toggle` is the explicit re-peek: it flashes without changing
/// which workspace is visible.
#[test]
fn hud_toggle_flashes_without_visibility_change() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.on_command(Command::HudToggle, None);
    let flashes = handle.hud_flashes();
    assert_eq!(flashes.len(), 1, "one explicit flash");
    assert_eq!(flashes[0][0].number, 1);
    assert_eq!(flashes[0][0].name, "Development");
    assert!(flashes[0][0].focused);
}

/// The prefix pin: arming flashes with the marker, the tick keeps the
/// pills up while the chord is armed, and they hide once disarmed and
/// expired.
#[test]
fn prefix_armed_pins_then_hides() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.set_hud_duration_for_test(1);

    // Arm the prefix the way the event tap would: same Arc it mutates,
    // then the armed event it would queue.
    let _ = sm.prefix.lock().unwrap().on_keydown(0x31, MODIFIER_ALT, Instant::now(), &|_, _| None);
    sm.on_prefix_armed();

    let flashes = handle.hud_flashes();
    assert!(!flashes.is_empty(), "arming flashes");
    assert!(flashes.last().unwrap()[0].prefix_armed, "armed marker on");

    // Repeated ticks while armed keep the pills from hiding.
    let hidden_before = handle.hud_hides();
    for _ in 0..5 {
        sm.on_tick();
    }
    assert_eq!(handle.hud_hides(), hidden_before, "armed prefix pins the HUD");

    // Disarm the chord (a fresh PrefixKey is disarmed) and let the
    // 1 ms deadline pass → one hide.
    *sm.prefix.lock().unwrap() = PrefixKey::new(PrefixConfig::default());
    std::thread::sleep(Duration::from_millis(10));
    sm.on_tick();
    assert_eq!(
        handle.hud_hides(),
        hidden_before + 1,
        "disarmed + expired → one hide"
    );

    // A lazy tick after hiding costs nothing.
    sm.on_tick();
    assert_eq!(
        handle.hud_hides(),
        hidden_before + 1,
        "no repeated hides"
    );
}

/// A disabled HUD never drew, never hid — the kill switch is total.
#[test]
fn disabled_hud_never_draws() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.set_hud_disabled_for_test();
    sm.on_command(Command::HudToggle, None);
    sm.on_prefix_armed();
    assert!(handle.hud_flashes().is_empty());
    assert_eq!(handle.hud_hides(), 0);
}

/// The default keybind table carries the recall chord (`alt-w`), and
/// nothing else shares it.
#[test]
fn hud_toggle_default_binding() {
    let binds = crate::config::keybinds::KeybindConfig::default();
    let hud = binds
        .bindings
        .iter()
        .find(|b| matches!(b.action, Command::HudToggle))
        .expect("hud-toggle has a default binding");
    assert_eq!(hud.keycode, 0x0D, "'w'");
    assert_eq!(hud.modifiers, MODIFIER_ALT);
    let clashes = binds
        .bindings
        .iter()
        .filter(|b| b.keycode == hud.keycode && b.modifiers == hud.modifiers)
        .count();
    assert_eq!(clashes, 1);
}
