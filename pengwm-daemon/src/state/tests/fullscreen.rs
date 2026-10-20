//! Fullscreen parking end-to-end through `StateManager`: the enter fast
//! paths (focus event, refused/failed write, discovery), the per-tick exit
//! probe, the shared reveal funnel, and the interactions with hiding,
//! popups, capacity, and the wake blackout.

use super::super::StateManager;
use super::common::*;
use crate::adapter::WindowClass;
use crate::adapter_test::Fault;
use crate::config::FullscreenStrategy;
use crate::state::store::RoutingVerdict;
use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;

/// Tile the harness windows through the normal discovery path so every
/// registration, class, and layout rule is the production one.
fn seed_tiles(sm: &mut StateManager) {
    sm.on_window_created(100, 42);
    sm.on_window_created(200, 42);
}

#[test]
fn a_live_element_outranks_a_transient_listing_omission() {
    // The live trace: during a macOS Space round-trip the apps answer
    // `kAXWindows` without their off-Space windows — visible tiled
    // members included. The listing diff must not judge that a death,
    // or every Space switch untracks the whole desktop and rediscovery
    // re-routes manually-moved windows onto the active workspace
    // (Firefox ws1→ws2, DrRacket ws3→ws1). The element read is the
    // discriminator: alive ⇒ artifact, dead ⇒ the untrack proceeds.
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    assert!(sm.workspaces[0].find_window(100).is_some());

    // The away period: the app answers, but omits both windows — while
    // their elements keep answering (the fake models the live element by
    // re-seeding the rect `unlist_window` drops).
    handle.unlist_window(42, 100);
    handle.unlist_window(42, 200);
    handle.inject_rect(100, Rect::new(5.0, 44.0, 900.0, 1000.0));
    handle.inject_rect(200, Rect::new(910.0, 44.0, 900.0, 1000.0));
    let pids: Vec<i32> = sm.store.all_window_pids().values().copied().collect();
    sm.sync_untracked_windows(pids);

    assert!(
        sm.store.contains(100) && sm.store.contains(200),
        "live elements were judged dead by a Space-transition listing omission"
    );
    assert!(
        sm.workspaces[0].find_window(100).is_some(),
        "the untrack destroyed the membership; rediscovery would re-route"
    );

    // A genuinely closed window is unlisted AND its element no longer
    // answers — the fast close-detector still fires. The fake models
    // the dead element: `unlist_window` drops the rect with the listing.
    handle.inject_window(42, 300);
    handle.unlist_window(42, 300);
    let pids: Vec<i32> = sm.store.all_window_pids().values().copied().collect();
    sm.sync_untracked_windows(pids);
    assert!(
        !sm.store.contains(300),
        "a dead element must still be judged dead on the omission"
    );
}

#[test]
fn fullscreen_parked_window_survives_a_transient_listing_miss_across_spaces() {
    // The user's repro: a window goes native fullscreen (its own macOS
    // Space), the user switches to another Space and back, and the window
    // comes back routed to whatever workspace is *active* — its old
    // membership gone. The suspected chain: while the window's Space is
    // inactive, its app's `kAXWindows` answers without it; the listed-set
    // sweep judged that a death and untracked the parked window
    // (verdict, pid map, everything), and the next sweep rediscovered it
    // fresh — routing to the active workspace, where the refused first
    // write parked it (the `FullscreenEnter: parking window (workspace
    // <active>)` log line).
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    assert!(sm.workspaces[0].find_window(100).is_some());

    // Native fullscreen: parked on its workspace.
    handle.inject_fullscreen_window(100);
    sm.on_window_focused(100);
    assert!(
        sm.store.is_parked(100) && !sm.store.is_hidden(100),
        "precondition: 100 is fullscreen-parked on its own workspace"
    );
    assert_eq!(
        sm.store.routing_verdict(100),
        Some(RoutingVerdict::Tiled),
        "precondition: the verdict outlives the tree slot"
    );

    // The Space switch: while 100's Space is inactive the app's listing
    // omits it, but its element still answers. The 2s sweep runs its
    // listing diff against that answer. (The fake models the live
    // element by re-seeding the rect `unlist_window` drops.)
    handle.unlist_window(42, 100);
    handle.inject_rect(100, Rect::new(5.0, 44.0, 1790.0, 1120.0));
    let pids: Vec<i32> = sm.store.all_window_pids().values().copied().collect();
    sm.sync_untracked_windows(pids);

    // A Space boundary is not a death: the park map owns this window's
    // membership until its reveal funnel (fullscreen exit probe or shown
    // event) says otherwise. Its death arrives via the destroyed
    // notification, app termination, or the dead-pid sweep.
    assert!(
        sm.store.contains(100),
        "a transient listing miss untracked a fullscreen-parked window"
    );
    assert!(
        sm.store.is_parked(100),
        "the untrack destroyed the park entry, losing the remembered workspace"
    );

    // And the Space switch back: the app lists the window again. With the
    // membership intact the sweep has nothing to rediscover — the window
    // is still parked where it was, not freshly routed onto the active
    // workspace.
    handle.inject_window(42, 100);
    let pids: Vec<i32> = sm.store.all_window_pids().values().copied().collect();
    sm.sync_untracked_windows(pids);
    assert!(
        sm.store.is_parked(100),
        "the rediscovery re-routed the window onto the active workspace"
    );
    assert_eq!(
        sm.store.routing_verdict(100),
        Some(RoutingVerdict::Tiled),
        "the rediscovery reset the routing verdict"
    );

    // Fullscreen exits: the probe reveals through the normal funnel.
    handle.clear_fullscreen_window(100);
    sm.on_fullscreen_exit(100);
    assert!(
        sm.workspaces[0].find_window(100).is_some(),
        "the window must reveal into the workspace it parked on"
    );
}

#[test]
fn focus_event_fullscreen_parks_and_siblings_reflow() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    assert!(sm.workspaces[0].find_window(100).is_some());

    // The fullscreened window keeps its old geometry (macOS owns it); the
    // sibling's tile grows to absorb the freed space.
    let sibling_before = handle.rect(200).unwrap();

    handle.inject_fullscreen_window(100);
    sm.on_window_focused(100);

    assert!(
        sm.workspaces.iter().all(|ws| ws.find_window(100).is_none()),
        "fullscreen window leaves the tree"
    );
    assert!(
        sm.store.is_fullscreen_parked(100),
        "remembered, not dropped"
    );
    assert!(
        sm.workspaces[0].find_window(200).is_some(),
        "sibling keeps its slot"
    );
    let sibling_after = handle.rect(200).unwrap();
    assert_ne!(
        sibling_after.width, sibling_before.width,
        "sibling reflowed"
    );
}

#[test]
fn unfullscreen_reveals_within_one_tick() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    handle.inject_fullscreen_window(100);
    sm.on_window_focused(100);
    assert!(sm.store.is_fullscreen_parked(100));

    handle.clear_fullscreen_window(100);
    sm.store.force_reconcile_for_test();
    sm.on_tick();

    assert!(
        sm.workspaces[0].find_window(100).is_some(),
        "revealed into its remembered workspace"
    );
    assert!(!sm.store.is_parked(100));
}

#[test]
fn unfullscreen_overflows_when_remembered_home_is_full() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    let max = sm.displays.max_tiles();
    // Fill workspace 1's remaining capacity (100 / 200 count toward it).
    let mut next = 300;
    while sm.workspaces[0].window_count() < max {
        handle.inject_window(43, next);
        sm.on_window_created(next, 43);
        next += 1;
    }
    handle.inject_fullscreen_window(100);
    sm.on_window_focused(100);
    assert!(sm.workspaces.iter().all(|ws| ws.find_window(100).is_none()));

    // Parked 100 freed one slot: refill workspace 1 to capacity again.
    handle.inject_window(43, 901);
    sm.on_window_created(901, 43);
    assert_eq!(sm.workspaces[0].window_count(), max, "home is full again");

    // Give the overflow a target: workspace 2 still has room.
    handle.inject_window(44, 999);
    sm.on_window_created(999, 44);
    assert!(sm.workspaces[1].find_window(999).is_some());

    handle.clear_fullscreen_window(100);
    sm.store.force_reconcile_for_test();
    sm.on_tick();

    assert!(
        sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()),
        "an unfullscreened window always becomes tileable"
    );
    assert!(
        sm.workspaces[0].find_window(100).is_none(),
        "home was at capacity — overflow applies"
    );
    assert!(
        sm.workspaces[1].find_window(100).is_some(),
        "overflow lands in the next workspace with room"
    );
    assert!(!sm.store.is_parked(100));
}

#[test]
fn hidden_while_fullscreen_upgrades_the_park_reason() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    handle.inject_fullscreen_window(100);
    sm.on_window_focused(100);
    assert!(sm.store.is_fullscreen_parked(100));

    // Minimize / app-hide lands while parked-fullscreen: reason upgrades,
    // idx preserved; a later unfullscreen probe must not reveal it.
    handle.inject_hidden_window(100);
    sm.on_window_hidden(100);
    assert!(!sm.store.is_fullscreen_parked(100));
    assert!(sm.store.is_hidden(100), "hide takes over the parked slot");

    handle.clear_fullscreen_window(100);
    sm.store.force_reconcile_for_test();
    sm.on_tick();
    assert!(
        !sm.workspaces.iter().any(|ws| ws.find_window(100).is_some()),
        "still hidden — the hidden flow owns the reveal"
    );

    // Un-minimize: the normal shown flow reveals it (not the fullscreen one).
    handle.clear_hidden_window(100);
    sm.on_window_shown(100);
    assert!(
        sm.workspaces[0].find_window(100).is_some(),
        "revealed by hide"
    );
    assert!(!sm.store.is_parked(100));
}

#[test]
fn fullscreen_parked_window_never_reenters_via_stray_shown() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    handle.inject_fullscreen_window(100);
    sm.on_window_focused(100);
    assert!(sm.store.is_fullscreen_parked(100));

    sm.on_window_shown(100);
    assert!(
        sm.workspaces.iter().all(|ws| ws.find_window(100).is_none()),
        "the exit probe owns the reveal"
    );
    assert!(sm.store.is_fullscreen_parked(100));
}

#[test]
fn strategy_tile_disables_every_probe() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    sm.set_fullscreen_strategy_for_test(FullscreenStrategy::Tile);

    handle.inject_fullscreen_window(100);
    sm.on_window_focused(100);
    handle.clear_fullscreen_window(100);
    sm.store.force_reconcile_for_test();
    sm.on_tick();

    assert!(
        sm.workspaces[0].find_window(100).is_some(),
        "legacy behavior: the window keeps its tree slot"
    );
    assert!(!sm.store.is_parked(100));
}

#[test]
fn failed_write_probes_fullscreen_and_parks() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    // The write is refused/failing (fullscreen rejects tile geometry — in
    // the wild as `kAXErrorFailure` Transient, not only a `Pinned` readback)
    // and the probe confirms the cause: one pass, parked — no backoff stint.
    handle.set_fault(100, Fault::Transient("kAXErrorFailure".into()));
    handle.inject_fullscreen_window(100);
    // The fullscreened window no longer sits at its tile — the drift is
    // what makes a write-attempt happen at all (the real daemon sees
    // `actual (0,39 1800x1130)` vs a half-width tile). The re-assert rides
    // the 2s misplaced sweep, exactly as the drifted window is re-checked in
    // production: age past the displacement grace, displace, sweep.
    handle.displace(100, 50.0, 50.0);
    sm.layout_cache
        .age_applied_for_test(100, std::time::Duration::from_secs(5));
    sm.force_window_sweep_for_test();
    sm.on_tick();

    assert!(
        sm.workspaces.iter().all(|ws| ws.find_window(100).is_none()),
        "failed write + fullscreen probe = immediate park"
    );
    assert!(sm.store.is_fullscreen_parked(100));
    assert!(
        sm.workspaces[0].find_window(200).is_some(),
        "siblings reflow the freed slot"
    );
}

#[test]
fn pinned_write_that_is_not_fullscreen_keeps_the_pin_backoff() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    // Zoomed / fixed-size / inactive Space / busy app: the probe answers
    // "no", the window stays in the tree and the existing pin machinery wins.
    handle.set_fault(100, Fault::Pinned);
    sm.apply_layout(0);
    sm.apply_layout(0);

    assert!(sm.workspaces[0].find_window(100).is_some());
    assert!(!sm.store.is_parked(100));
}

#[test]
fn discovery_probe_parks_window_created_fullscreen() {
    let (mut sm, handle) = setup_with_handle(1);
    // Daemon restart (or created-straight-to-fullscreen): the window is
    // discovered already fullscreen — no persistence, self-heals at
    // discovery instead of writing into it first.
    handle.inject_window(44, 500);
    handle.inject_fullscreen_window(500);
    sm.on_window_created(500, 44);

    assert!(
        sm.workspaces.iter().all(|ws| ws.find_window(500).is_none()),
        "never given a live tile"
    );
    assert!(sm.store.is_fullscreen_parked(500));
}

#[test]
fn popup_fullscreen_is_ignored() {
    let (mut sm, handle) = setup_with_handle(1);
    // A popup class at creation.
    handle.inject_window_kind(300, WindowClass::Floating);
    handle.inject_rect(300, Rect::new(960.0, 540.0, 400.0, 300.0));
    handle.inject_fullscreen_window(300);
    sm.on_window_created(300, 42);
    assert!(sm.workspaces[0].is_popup(300), "popup routing unchanged");
    assert!(!sm.store.is_parked(300), "parking never applies to popups");

    // And a fullscreened already-placed popup stays put.
    sm.on_window_focused(300);
    assert!(sm.workspaces[0].is_popup(300));
    assert!(!sm.store.is_parked(300));
}

#[test]
fn idle_exit_probe_does_not_touch_tree_windows() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    // No parked windows at all: the probe is a no-op even when the fake
    // reports fullscreen on a live tree member (tree members are never
    // probed per tick — only focused/failed-write/creation paths consult
    // them).
    handle.inject_fullscreen_window(200);
    sm.store.force_reconcile_for_test();
    sm.on_tick();
    assert!(sm.workspaces[0].find_window(200).is_some());
    assert!(!sm.store.is_parked(200));
}

#[test]
fn parked_window_survives_the_unlisting_sweep() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    handle.inject_fullscreen_window(100);
    sm.on_window_focused(100);
    assert!(sm.store.is_fullscreen_parked(100));

    // The app still lists its own fullscreen window (native fullscreen does
    // not remove it from the app's AX window set), so the 2s sweep leaves
    // the parked entry alone.
    sm.force_window_sweep_for_test();
    sm.on_tick();
    assert!(sm.store.is_fullscreen_parked(100));
    assert!(sm.store.contains(100));
}

#[test]
fn wake_blackout_suppresses_the_exit_probe() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    handle.inject_fullscreen_window(100);
    sm.on_window_focused(100);
    assert!(sm.store.is_fullscreen_parked(100));

    // The real blackout answers *every* AX attribute as unreadable (→
    // "not fullscreen"); the suppression, like the listing sweep's, lives at
    // the reconcile level.
    sm.on_system_woke();
    handle.set_ax_blackout(true);
    handle.clear_fullscreen_window(100);
    sm.store.force_reconcile_for_test();
    sm.on_tick();
    assert!(
        sm.store.is_fullscreen_parked(100),
        "no judgment while the wake resync is pending"
    );
}

#[test]
fn closed_while_fullscreen_parked_untracks_normally() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    handle.inject_fullscreen_window(100);
    sm.on_window_focused(100);
    assert!(sm.store.is_fullscreen_parked(100));

    // The app closes it. A real close kills the element, so the
    // fullscreen attribute stops answering — clear the mark too (the
    // listing diff's probe reads that attribute to tell a dead window
    // from one parked in an inactive Space).
    handle.unlist_window(42, 100);
    handle.clear_fullscreen_window(100);
    sm.force_window_sweep_for_test();
    sm.on_tick();

    assert!(!sm.store.contains(100));
    assert!(sm.store.fullscreen_parked().is_empty());
}

/// Both reasons coexist in the one parked map, and each reveals through its
/// own flow.
#[test]
fn park_reasons_coexist_side_by_side() {
    let (mut sm, handle) = setup_with_handle(1);
    seed_tiles(&mut sm);
    handle.inject_fullscreen_window(100);
    sm.on_window_focused(100); // parks 100 (Fullscreen)
    handle.inject_hidden_window(200);
    sm.on_window_hidden(200); // parks 200 (Hide)

    let snapshot: Vec<WindowId> = sm.store.fullscreen_parked();
    assert_eq!(snapshot, vec![100]);
    assert!(sm.store.is_hidden(200));
    assert!(sm.store.is_parked(100) && sm.store.is_parked(200));
}
