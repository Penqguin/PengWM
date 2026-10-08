# Distribution (bare binaries + LaunchAgent + Homebrew)

Prebuilt, checksummed artifacts: a **flat binary tarball** (primary — two
executables installed to `~/.pengwm/bin`, `pengwm` CLI symlink on PATH,
launchd LaunchAgent runs the daemon at login). No App Store, no
auto-updater, no `.app` bundle.

> **Why no bundle:** the project is certificate-less, and under ad-hoc
> signing a bundle buys nothing — TCC's designated requirement for an
> ad-hoc signature degrades to a per-build cdhash, so *every update
> re-prompts for Accessibility regardless of layout*. ADR-0001
> (`docs/adr/0001-bare-binary-distribution.md`) records the full reasoning
> and the accepted trade-off: **one Accessibility re-grant per update,
> documented loudly**. A Developer ID certificate would remove that cost
> and re-enable notarization; the signing checklist below is ready if that
> decision is ever revisited.

Source builds (`--from-source`, developers only) are ad-hoc signed the same
way: every rebuild costs one re-grant. Running a dev build from a terminal
does not re-prompt because TCC attributes terminal-launched binaries to the
terminal app — don't use that to predict launchd behavior, where the daemon
stands alone. Dev checkouts run outside the blessed install root and must
opt in with `PENGWM_DEV=1`.

## The install layout

```
$PENGWM_HOME/bin/pengwm           # daemon + CLI (PENGWM_HOME default ~/.pengwm)
$PENGWM_HOME/bin/pengwm-menubar   # NSStatusItem, spawned by the daemon
$PREFIX/pengwm -> $PENGWM_HOME/bin/pengwm       # CLI symlinks
$PREFIX/pengwm-menubar -> ...                   # (default $PREFIX /usr/local/bin,
                                                #  falls back to ~/.local/bin)
~/Library/LaunchAgents/com.pengwm.daemon.plist  # ProgramArguments = the bin-dir
                                                # daemon directly (sibling lookup
                                                # finds the menubar next to it)
~/Library/Logs/pengwm.log                       # agent stdout/stderr
```

`PENGWM_HOME` is the one layout seam: `install.sh --home DIR` installs to
`DIR/bin` and persists `PENGWM_HOME=DIR` in the LaunchAgent's
`EnvironmentVariables` so the daemon's one-copy policy (`location.rs`)
resolves the same root. Homebrew Cellar pengwm paths are also blessed by
the policy so a `brew install pengwm` copy needs no config.

## What CI produces

On every `v*` tag, `.github/workflows/release.yml` builds both binaries for
each architecture and uploads, per arch:

```
pengwm-<tag>-<target>.tar.gz      (+ .sha256)   # the one install artifact
```

`<target>` is `aarch64-apple-darwin` or `x86_64-apple-darwin`. The tarball
contains the two binaries + LICENSE at its root. install.sh verifies the
sidecar, `codesign --verify` each binary (hard fail), and soft-warns when
`spctl` rejects the build (the expected ad-hoc case).

## Signing modes

| Secrets configured | `codesign` behavior | User experience |
|---|---|---|
| `APPLE_CERTIFICATE_P12` + `APPLE_DEVELOPER_IDENTITY` | Developer ID, `--options runtime --timestamp` | Stable identity, grant survives updates. |
| none | ad-hoc (`codesign -s -`) | Installs and runs, but every update looks like a new binary → **one Accessibility re-grant per update** (accepted by ADR-0001). |

No special entitlements are needed for AX/CGEventTap — those are TCC grants
in System Settings, not entitlements. (The old `packaging/entitlements.plist`
was deleted with the bundle; hardened-runtime signing can re-add minimal
entitlements if a Developer ID is ever configured.)

## Enabling Developer-ID signing (optional, not currently used)

1. Apple Developer Program ($99/yr). Create a **Developer ID Application**
   certificate, export the `.p12`.
2. Add repo/actions secrets:
   - `APPLE_CERTIFICATE_P12` — base64 of the `.p12`
     (`base64 -i cert.p12 | pbcopy`)
   - `APPLE_CERTIFICATE_PASSWORD` — `.p12` export password
   - `APPLE_DEVELOPER_IDENTITY` — e.g. `Developer ID Application: Your Name (TEAMID)`
   - `APPLE_TEAM_ID`
3. Tag a release and check the workflow log for `Signing with Developer ID`.
   Notarization (optional, needs the API-key secrets + a zip artifact) was
   removed from the workflow with the bundle; re-add `xcrun notarytool`
   steps if Gatekeeper prompting matters.

## install.sh behavior

```
./install.sh                    # latest release → $PENGWM_HOME/bin (+ CLI symlinks + agent)
./install.sh --version v0.6.0   # pinned version
./install.sh --home DIR         # install root override (binaries in DIR/bin)
./install.sh --prefix DIR       # where the CLI symlinks go
./install.sh --from-source      # (developers) cargo build → install from target/release
./install.sh --repo OWNER/REPO  # fork testing
```

At install time it also **retires the old v0.5.x `.app` layout**: any
`/Applications/PengWM.app` or `~/Applications/PengWM.app` is booted out and
removed (skipped under `PENGWM_DEV=1`), and reports a daemon running from a
non-target path (typically started by hand or an IDE from a checkout).
`PENGWM_DEV=1` suppresses checkout warnings for developer rebuilds.

Support-burden note: migrating from the bundle layout costs **one
Accessibility re-grant** (the TCC entry is bound per layout: bundle id vs
binary path). Same grant count as a signing-identity change.

## Homebrew

- **`Formula/pengwm.rb`** (Penqguin/homebrew-tap) — the blessed brew path:
  `bin.install pengwm, pengwm-menubar`, with caveats covering the
  Accessibility grant, the per-upgrade re-grant, and LaunchAgent setup.
  `brew install penqguin/tap/pengwm`.
- **`Casks/pengwm.rb`** (tap) — tombstoned by the bump workflow (first run
  after the cask retired): `disable!` pointing at the formula.

Formula version + sha256 bumps are automated; see the next section.

## Homebrew bump automation

`.github/workflows/homebrew-bump.yml` keeps the tap in sync after every
release:

1. Triggered by the release workflow after a `v*` release publishes.
2. Clones the tap repo (`Penqguin/homebrew-tap`).
3. Updates `Formula/pengwm.rb`: `version` and the per-arch `sha256` lines,
   taken from the release's `.sha256` sidecars — no manual bumping.
4. Tombstones `Casks/pengwm.rb` (once, idempotent — later bumps skip it).
5. Opens a PR on the tap for a human to review and merge.

Authentication uses a PAT secret named `HOMEBREW_TAP_TOKEN`. If the PR
doesn't show up, check the workflow run on the release, re-run it, or do a
one-off manual bump.

## Migrating from the v0.5.x bundle / source builds

`cargo build` users and bundle installs just re-run `./install.sh`. The
`pengwm` CLI path (`$PREFIX/pengwm`) stays a symlink — it now points into
`$PENGWM_HOME/bin` — and the signing mode is unchanged (ad-hoc), so **one
Accessibility re-grant** is expected (System Settings → Accessibility → add
`~/.pengwm/bin/pengwm`). After that, every update keeps costing exactly
that one re-grant.
