# ADR-0001: Bare-binary distribution

Date: 2026-10-07
Status: Accepted
Revises: the v0.5.1 bundle switch (`a53ac21`) and interacts with the
certificate-less signing decision (`30d3828`).

## Context

PengWM ships two executables (`pengwm` daemon/CLI, `pengwm-menubar`
NSStatusItem) plus a launchd LaunchAgent. Since v0.5.1 the release artifact
is a signed `PengWM.app` bundle installed to `/Applications`, adopted to
give TCC a stable attribution target (`CFBundleIdentifier
com.pengwm.daemon`), provide Launchpad/Spotlight presence, and enable a
Homebrew cask.

Two constraints make that bundle hard to justify:

1. **The project is certificate-less** (no Apple Developer Program). Under
   ad-hoc signing, TCC's designated requirement degrades to a bare cdhash —
   a hash of the binary contents. Every ad-hoc rebuild/update is a
   brand-new program to TCC, and the bundle identifier never enters the DR
   for ad-hoc binaries. Measured consequence (already conceded in
   `docs/distribution.md`): *every update of the ad-hoc bundle re-prompts
   for Accessibility anyway*. The bundle's headline benefit — "a grant made
   once survives every update" — is only available with a Developer ID
   certificate, which we have decided not to buy.
2. **PengWM is a terminal-native tiling WM.** Its users install via
   `curl | sh` or Homebrew and control the daemon through the `pengwm` CLI.
   Launchpad presence buys them little; the bundle's ceremony (Info.plist,
   icns, nested signing, `ditto`, `lsregister -f`, stray-bundle hunts, four
   CI artifacts per arch, cask + tombstone formula) is cost without
   leverage.

Terminal-run dev builds never re-prompt, but that observation does not
transfer to launchd: TCC attributes a terminal-launched binary's request to
the terminal app. Launchd-run processes stand alone.

## Decision

Distribute **bare, ad-hoc-signed binaries** and retire the `.app` bundle:

- **Install root:** `$PENGWM_HOME/bin` (default `~/.pengwm/bin`). The
  daemon, menubar, CLI symlinks, and LaunchAgent all resolve against this
  one root.
- **One-copy policy kept, restated:** the daemon refuses to run from
  outside the blessed install root (or a Homebrew Cellar pengwm path), so
  exactly one daemon copy ever fights for the IPC socket. `PENGWM_DEV=1`
  still opts dev checkouts in.
- **Accepted cost, documented loudly:** with no signing certificate, every
  release update — and every `brew upgrade` — costs each user **one
  Accessibility re-grant**. This is parity, not regression: the ad-hoc
  bundle re-prompts per update too. The installer prints the warning;
  README and this ADR record it.
- **Homebrew:** the real Formula returns (`bin.install` both binaries);
  the cask and its tombstone formula are retired.
- **Hard cut:** the next release's `install.sh` migrates in place
  (bootout + remove any `PengWM.app`, install flat) and the flat tarball is
  the only artifact (`.tar.gz` + `.sha256` per arch; the zips existed only
  for notarytool, which we do not use).

## Consequences

- `packaging/` (app-template, icns, `make_app.sh`, entitlements), the
  committed `PengWM.app`, and the cask are deleted; the installer and
  LaunchAgent point at the install root.
- The LaunchServices/`lsregister` attribution gotcha and the Spotlight
  fake-app hygiene issue are bundle-only diseases and disappear with the
  bundle.
- The TCC pane will show the bare binaries without app icons/names. We
  accept this; a Developer ID certificate would fix naming, notarization,
  and grant persistence at once, and remains a documented future option
  (`docs/distribution.md` "Signing").
- Future layout changes reduce to one config point (the install root) plus
  the installer, instead of plist/signing/packaging machinery.
