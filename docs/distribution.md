# Distribution (prebuilt binaries + `PengWM.app` + Homebrew)

Prebuilt, signed artifacts: a **`PengWM.app` bundle** (primary — installs to
`/Applications`, shows in Launchpad, CLI via a shim), plus a legacy
flat-binary tarball (Homebrew formula + `install.sh --no-app`). No App
Store, no auto-updater — just: download a stable-signed build so
Accessibility grants survive updates, without requiring Rust.

An ad-hoc signed build installs and runs fine, but every update looks like a
new binary and macOS re-prompts for Accessibility — the `.app` bundle keeps
this a little better than the flat layout (TCC attributes the grant to the
bundle, `com.pengwm.daemon`), but ad-hoc signatures still can't pin it.

> **The headline caveat:** PengWM currently ships **ad-hoc signed** builds
> (the maintainer has opted to stay certificate-less for now). If that ever
> becomes a real support burden, the fix is a $99/yr Apple Developer Program
> account + the secrets below — the pipeline flips to stable Developer-ID
> signing automatically, with no other changes.

## The bundle (`packaging/`)

`packaging/app-template/Contents/Info.plist` is the bundle definition
(the `Info.plist` lives in a non-`.app` directory on purpose: Spotlight
indexes any `.app`-shaped folder it finds, and a template with no
executables surfaced as a fake third PengWM in Spotlight):
`CFBundleIdentifier` `com.pengwm.daemon` (same as the LaunchAgent label),
`CFBundleExecutable` `pengwm`, **`LSUIElement`** (no Dock icon — the WM's UI
surfaces are the bar/menubar child processes) and `CFBundleIconFile`
`PengWM.icns`. The icon is generated from the igloo penguin SVG; regeneration
recipe (any machine with librsvg + imagemagick + iconutil):

```sh
rsvg-convert -w 1024 -h 1024 <igloo-logo.svg> -o base.png   # render once at 1024
for s in 16 32 128 256 512; do                              # iconset sizes
  magick base.png -resize ${s}x${s}   icon_${s}x${s}.png
  magick base.png -resize $((s*2))x$((s*2)) icon_${s}x${s}@2x.png
done
iconutil -c icns AppIcon.iconset -o PengWM.icns             # copy the ten pngs into AppIcon.iconset first
```

`packaging/make_app.sh [dist-dir]` assembles `PengWM.app/Contents/MacOS/`
from release binaries, stamps the bundle version from `pengwm --version`,
and signs nested executables first, then the bundle (outermost last), with
`PENGWM_SIGN_IDENTITY` when set (Developer ID in CI) or ad-hoc otherwise.
It exists so local source builds produce the same bundle as CI.

Note for maintainers: **GitHub release asset URLs match
case-insensitively**, so the bundle artifacts are named `pengwm-app-*`
(never `PengWM-*`) to avoid ambiguous collisions with the legacy
`pengwm-<tag>-<target>` tarballs.

## The penqguin.com front door (`edge/`)

`pengwm.penqguin.com` is a Workers **Custom Domain** (Auto DNS/cert created by
Wrangler on first deploy) bound to the one-file Worker in `edge/`:

| URL target | Redirect (302) |
|---|---|
| `https://pengwm.penqguin.com/install.sh` | `raw.githubusercontent.com/Penqguin/PengWM/main/install.sh` |
| `https://pengwm.penqguin.com/uninstall.sh` | `raw.githubusercontent.com/Penqguin/PengWM/main/uninstall.sh` |
| anything else | `https://github.com/Penqguin/PengWM` |

The scripts live in **this repo only** — the Worker is a pure redirect, so
there is no copy to keep in sync, and the redirect target follows `main`
(meaning no Cloudflare change is needed per release; the script resolves the
latest GitHub release at install time). Deploy is manual and rare:

```sh
cd edge && npm install && npm run deploy   # one-time `wrangler login` required
```

Do not add an auto-deploy workflow for this: the Worker changes next-to-never,
and it would add a `CLOUDFLARE_API_TOKEN` secret to rotate. Everything
Cloudflare-adjacent about penqguin.com's main site (`igloo` repo) is untouched
by this Worker — they share the zone, not the deployment.

## What CI produces

On every `v*` tag, `.github/workflows/release.yml` builds all three binaries
for both architectures and uploads, per arch:

```
pengwm-app-<tag>-<target>.tar.gz (+ .sha256)   # PengWM.app bundle, primary
pengwm-app-<tag>-<target>.zip     (+ .sha256)  # PengWM.app bundle, cask + notarization
pengwm-<tag>-<target>.tar.gz      (+ .sha256)  # legacy flat binaries (formula, --no-app)
pengwm-<tag>-<target>.zip         (+ .sha256)  # legacy flat binaries, notarytool-compatible
```

The bundle tarball is the primary install artifact (`install.sh`, cask). The
zips exist because `notarytool submit` accepts dmg/pkg/zip, not tar.gz; when
notarized they are also stapled (`xcrun stapler staple`). The flat layout is
kept for two consumers: `Formula/pengwm.rb`'s `bin.install` needs loose
binaries, and `install.sh --no-app` selects it explicitly.

## Signing modes

| Secrets configured | `codesign` behavior | User experience |
|---|---|---|
| `APPLE_CERTIFICATE_P12` + `APPLE_DEVELOPER_IDENTITY` | Developer ID, `--options runtime --timestamp`, `packaging/entitlements.plist` | Stable identity, grant survives updates. Notarized too if API key present — no Gatekeeper warning. |
| none | ad-hoc (`codesign -s -`) | Installs and runs, but every update looks like a new binary → macOS re-prompts for Accessibility. |

No special entitlements are needed for AX/CGEventTap — those are TCC grants
in System Settings, not entitlements. `packaging/entitlements.plist` is
intentionally minimal (hardened runtime compatibility).

## Enabling Developer-ID signing + notarization (optional, not currently used)

1. Apple Developer Program ($99/yr). Create a **Developer ID Application**
   certificate, export the `.p12`.
2. App Store Connect API key (Issuer ID + Key ID + `.p8`) for notarization.
3. Add repo/actions secrets (all 7 — signing and notarization are independent,
   and the core promise of "Accessibility grants survive updates" needs at
   minimum the certificate half):

   - `APPLE_CERTIFICATE_P12` — base64 of the `.p12`
     (`base64 -i cert.p12 | pbcopy`)
   - `APPLE_CERTIFICATE_PASSWORD` — `.p12` export password
   - `APPLE_DEVELOPER_IDENTITY` — e.g. `Developer ID Application: Your Name (TEAMID)`
   - `APPLE_TEAM_ID`
   - `APPLE_API_KEY_P8` — base64 of the `.p8`
   - `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`

4. Tag a release (`git tag v0.6.0 && git push origin v0.6.0`) and check the
   workflow log for `Signing with Developer ID` + notarytool acceptance.
5. Bump `Formula/pengwm.rb` (version/URLs/sha256s) per the maintainer note in
   the formula.

## install.sh behavior

```
./install.sh                    # latest PengWM.app → /Applications (+ CLI shim)
./install.sh --version v0.5.0   # pinned version
./install.sh --app-dir DIR      # install the bundle somewhere else
./install.sh --no-app           # legacy flat layout: loose binaries in --prefix
./install.sh --from-source      # cargo build → bundle (ad-hoc signed, re-prompts)
./install.sh --repo OWNER/REPO  # fork testing
```

The default is the bundle: it extracts the release's `pengwm-app-*` tarball
into `/Applications` (`~/Applications` when not writable), symlinks `pengwm`/`pengwm-menubar` into the prefix, and the LaunchAgent
runs the bundle's daemon binary directly (so `current_exe()` sibling lookup
still finds the menubar in `Contents/MacOS`). Releases older than the
bundle switch-over fall back to the flat layout with a note. It verifies
`shasum -a 256` against the `.sha256` sidecar when present, runs
`codesign --verify` (hard fail), and warns (soft) when Gatekeeper (`spctl`)
doesn't trust the build — i.e. ad-hoc vs notarized.

Support-burden note: switching a Machine between flat and bundle layouts means
**one Accessibility re-grant** (the TCC entry is bound per layout: old path vs
bundle id). Same grant count as a layout-independent signing-identity change.

**LaunchServices + TCC attribution gotcha (fixed in install.sh):** replacing
`PengWM.app` on disk (an update via `ditto`) orphans the bundle's
LaunchServices registration. A bundle LaunchServices cannot attribute will
fail `AXIsProcessTrusted()` under launchd **even with a fresh grant** —
System Settings shows PengWM in Accessibility, the daemon still says
"needs Accessibility", while running the binary by hand works. `install.sh`
now runs `lsregister -f` on the installed bundle (the same fix `open
PengWM.app` used to apply as a side effect); if a manual install ever shows
this pattern, register the bundle or `open` it once.

## Homebrew

Two artifact styles:

- **`Casks/pengwm.rb`** — the `.app` cask (`app PengWM.app` + a `binary` for
  the CLI). This is what most users should get: `brew tap penqguin/tap &&
  brew install --cask pengwm`.
- **`Formula/pengwm.rb`** — the legacy flat-binary formula (kept for the
  `bin.install` workflow and anyone who prefers no `/Applications` install).

After tagging:

1. Download the bundle tarballs, read their `.sha256` sidecars.
2. Bump `version` + both `sha256` lines in `Casks/pengwm.rb` (same drill for
   the formula if the flat layout is still being updated).
3. `brew info --cask penqguin/tap/pengwm` to parse-check, then commit both
   files to the tap repo (`homebrew-tap`: `Formula/` + `Casks/`).

## Migrating from source builds

## Migrating from source builds / flat installs

`cargo build` users and existing flat-layout installs just re-run
`./install.sh`. The `pengwm` CLI path (`$PREFIX/pengwm`) stays the same —
it becomes a symlink into the bundle — but the daemon now lives in
`PengWM.app`, and the signing identity changed, so **one Accessibility
re-grant** is expected (System Settings → Accessibility → add PengWM).
After that, updates keep the grant. To stay on the flat layout, use
`--no-app`.
