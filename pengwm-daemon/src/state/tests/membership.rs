//! Characterization of transition ordering across membership and placement.

use super::common::*;
use crate::adapter::DisplayInfo;
use pengwm_core::command::Command;
use pengwm_core::tree::Direction;
use pengwm_core::workspace::PendingKind;
use std::time::Duration;

#[test]
fn explicit_transfer_promotes_source_and_preserves_destination_opening_order() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.displays.set_max_tiles(1);
    for id in [100, 200, 300] {
        sm.on_window_created(id, 42);
    }
    sm.workspaces[1].add_window(900, None);
    let order = sm.store.opening_order(100).unwrap();
    sm.transfer_window(0, 1, 100);

    assert!(sm.workspaces[0].find_window(200).is_some());
    assert!(sm.workspaces[0].pending_window(300).is_some());
    assert!(!sm.workspaces[0].owns(100));
    let moved = sm.workspaces[1].pending_window(100).unwrap();
    assert_eq!(moved.kind, PendingKind::Explicit);
    assert_eq!(moved.order, order);
    assert_eq!(sm.where_should(100, 42), Some(1));
    assert_eq!(
        handle.rect(200),
        sm.workspaces[0]
            .layout(sm.gap_inner, sm.gap_outer)
            .get(&200)
            .copied(),
        "the freed source slot is promoted and placed during the transfer"
    );
    sm.assert_identity_invariant();
}

#[test]
fn hidden_pending_roundtrip_preserves_captured_intent_and_order() {
    let (mut sm, _) = setup_with_handle(1);
    sm.displays.set_max_tiles(1);
    for id in [100, 200, 300] {
        sm.on_window_created(id, 42);
    }
    let original = sm.workspaces[0].pending_window(200).unwrap();

    assert!(sm.park_hidden_member(200));

    assert!(!sm.workspaces[0].owns(200));
    assert_eq!(sm.store.parked_entry(200).unwrap().pending, Some(original));
    assert_eq!(sm.where_should(200, 42), Some(0));
    sm.assert_identity_invariant();

    assert!(sm.return_hidden_member(200));

    assert!(!sm.store.is_parked(200));
    assert_eq!(sm.workspaces[0].pending_window(200), Some(original));
    assert!(sm.workspaces[0].find_window(100).is_some());
    assert!(sm.workspaces[0].pending_window(300).is_some());
    sm.assert_identity_invariant();
}

#[test]
fn detached_overflow_rehomes_while_gated_and_places_only_after_return() {
    let (mut sm, handle) = setup_with_handle(2);
    sm.displays.set_max_tiles(1);
    sm.on_command(
        Command::FocusDisplay {
            direction: Direction::Right,
        },
        None,
    );
    for id in [300, 400, 500] {
        handle.inject_window(42, id);
        sm.on_window_created(id, 42);
    }
    let traveling = sm.workspaces[1].pending_window(400).unwrap();
    sm.on_window_hidden(500);
    handle.set_first_desktop(1, false);
    sm.refresh_space_gate_for_test();
    let writes = handle.writes();
    handle.set_displays(vec![DisplayInfo {
        id: 1,
        origin: (0, 0),
        size: (1920, 1080),
    }]);

    sm.on_monitor_removed(2);

    assert_eq!(sm.workspaces[0].pending_window(400), Some(traveling));
    assert_eq!(sm.store.parked_idx(500), Some(0));
    assert!(sm.workspaces[1].find_window(300).is_some());
    assert_eq!(sm.where_should(400, 42), Some(0));
    assert_eq!(
        handle.writes(),
        writes,
        "recovery is bookkeeping while gated"
    );

    handle.set_first_desktop(1, true);
    sm.refresh_space_gate_for_test();
    assert!(sm.workspaces[0].find_window(400).is_some());
    assert!(handle.writes() > writes);
    sm.assert_identity_invariant();
}

#[test]
fn layout_triggered_removal_promotes_pending_and_retires_the_gone_element() {
    let (mut sm, handle) = setup_with_handle(1);
    sm.displays.set_max_tiles(1);
    for id in [100, 200, 300] {
        sm.on_window_created(id, 42);
    }
    sm.os.close_window(100);
    handle.set_fault(100, Fault::Gone);
    sm.layout_cache
        .age_applied_for_test(100, Duration::from_secs(5));
    sm.layout_cache.note_displaced(100, None);
    sm.apply_layout(0);
    sm.layout_cache
        .age_gone_for_test(100, Duration::from_secs(1));

    sm.apply_layout(0);

    assert!(!sm.store.contains(100));
    assert!(handle.forgotten().contains(&100));
    assert!(sm.workspaces[0].find_window(200).is_some());
    assert!(sm.workspaces[0].pending_window(300).is_some());
    assert_eq!(
        handle.rect(200),
        sm.workspaces[0]
            .layout(sm.gap_inner, sm.gap_outer)
            .get(&200)
            .copied()
    );
    assert_eq!(
        sm.where_should(100, 42),
        Some(0),
        "indirect removal retains rediscovery trust"
    );
    sm.assert_identity_invariant();
}
