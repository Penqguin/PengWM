# Troubleshooting

Symptom → cause → fix. Run the daemon with `RUST_LOG=debug` to see window
creation, focus changes, keybind matches, and layout decisions while you
diagnose.

## PengWM doesn't manage any windows

**Cause:** missing Accessibility permissions, or they were granted to the
wrong binary (e.g. your terminal instead of `pengwm`, or a stale build
after `cargo build` replaced the binary — macOS ties the grant to the
binary, so every rebuild re-prompts).

**Fix:** System Settings → Privacy & Security → Accessibility → add the
`pengwm` binary (or your terminal, if you launch it from there). The daemon
exits with instructions when permissions are missing — check the first log
lines.

## Keybinds do nothing

**Cause:** the CGEventTap needs the same Accessibility grant, and an
invalid action in config.toml fails the whole keybind load (it falls back
to defaults with a warning).

**Fix:** check the logs for the keybind-load warning, verify action names
against the [configuration](configuration.md) table, and confirm the
grant above.

## An app's windows never tile

**Cause:** the app is in `restricted_apps` — those apps are overlay apps:
every window pops out as a centered overlay instead of tiling. If that's
not what you want, remove the bundle id and reload.

**Fix:** remove the bundle id from `restricted_apps` and reload. True
dialog/panel windows (dialogs, PiP, `AXFloatingWindow` panels) intentionally
never tile either — they render as centered popups; see
[Popups](configuration.md#popups). Sheets and unknown-subrole windows stay
unmanaged. If a window is never seen at all, check `RUST_LOG=debug` for
whether a `WindowCreated` notification arrived.

## Firefox (or another busy app) stops responding to layout

**Cause:** the app isn't converging on the requested rects. After 3
consecutive pinned writes PengWM backs off for 15s rather than rewriting
every tick — this is intentional, not a stall.

**Fix:** wait; the retry is automatic. If a window is *permanently* stuck,
it may be live-resizing or holding a modal state — release it and the next
layout pass re-tiles.

## Windows take ~seconds to re-tile after sleep/wake

**Cause:** macOS answers no Accessibility queries for seconds after wake.
PengWM arms a resync on wake and commits it once polls return windows
(up to a 20s deadline), rather than writing through dead element
references.

**Fix:** wait a few seconds. If windows are still stranded after that,
`pengwm reveal-all` re-tiles everything tracked as hidden.

## Workspaces look wrong after docking/undocking

**Cause:** monitor topology changed. Removed outputs' workspaces move to
primary (hidden there); a new output shows the first hidden workspace.

**Fix:** switch workspaces to pull them where you want them (`workspace-N`
swaps a workspace onto the focused monitor). If the saved session keeps
restoring a stale topology, `pengwm clear-session` resets to config
defaults on next launch.

## Windows are stranded offscreen (daemon died mid-hide)

**Cause:** hidden workspaces park windows offscreen, and a crash between
hide and re-tile can strand them there.

**Fix:** `pengwm reveal-all` re-tiles every hidden window. The bottom-edge
hide strip also stays visible in Mission Control as an escape hatch: you
can drag a stranded window back by hand.

## "Displays have separate Spaces" keeps coming up

**Cause:** macOS Spaces constrains one display's layout to affect the
other when this is off; PengWM assumes per-display independence.

**Fix:** System Settings → Desktop & Dock → turn on *Displays have
separate Spaces*, then restart the daemon.
