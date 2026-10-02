# Distribution (prebuilt tarballs + Homebrew)

Prebuilt, signed tarballs + Homebrew. No `.app` bundle, no App Store, no
auto-updater — just: download a stable-signed build so Accessibility grants
survive updates, without requiring Rust.

> **The headline caveat first:** stable signatures only exist once the
> maintainer configures Apple Developer-ID secrets (see below). Until then,
> CI ships ad-hoc signed builds: they install and run fine, but every update
> looks like a new binary and macOS re-prompts for Accessibility — the exact
> problem this pipeline exists to fix.

## What CI produces

On every `v*` tag, `.github/workflows/release.yml` builds all three binaries
for both architectures and uploads four assets per arch:

```
pengwm-<tag>-aarch64-apple-darwin.tar.gz (+ .sha256)
pengwm-<tag>-aarch64-apple-darwin.zip     (+ .sha256, notarytool-compatible)
pengwm-<tag>-x86_64-apple-darwin.tar.gz  (+ .sha256)
pengwm-<tag>-x86_64-apple-darwin.zip     (+ .sha256, notarytool-compatible)
```

The tarball is the install artifact (`install.sh`, Homebrew). The zip exists
because `notarytool submit` accepts dmg/pkg/zip, not tar.gz; when notarized
it is also stapled (`xcrun stapler staple`) so offline Gatekeeper checks pass.

## Signing modes

| Secrets configured | `codesign` behavior | User experience |
|---|---|---|
| `APPLE_CERTIFICATE_P12` + `APPLE_DEVELOPER_IDENTITY` | Developer ID, `--options runtime --timestamp`, `packaging/entitlements.plist` | Stable identity, grant survives updates. Notarized too if API key present — no Gatekeeper warning. |
| none | ad-hoc (`codesign -s -`) | Installs and runs, but every update looks like a new binary → macOS re-prompts for Accessibility. |

No special entitlements are needed for AX/CGEventTap — those are TCC grants
in System Settings, not entitlements. `packaging/entitlements.plist` is
intentionally minimal (hardened runtime compatibility).

## Enabling Developer-ID signing + notarization

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
./install.sh                    # latest prebuilt for current arch
./install.sh --version v0.5.0   # pinned version
./install.sh --from-source      # cargo build (ad-hoc signed, re-prompts)
./install.sh --repo OWNER/REPO  # fork testing
```

It verifies `shasum -a 256` against the `.sha256` sidecar when present,
runs `codesign --verify` (hard fail), and warns (soft) when Gatekeeper
(`spctl`) doesn't trust the build — i.e. ad-hoc vs notarized.

## Homebrew

`Formula/pengwm.rb` tracks releases with per-arch URLs. After tagging:

1. Download both tarballs, read their `.sha256` files.
2. Bump `version` + both `sha256` lines in the formula.
3. `brew install --build-from-source Formula/pengwm.rb` to test, then commit.

For a wider audience, move the formula to a `homebrew-tap` repo
(`brew tap penqguin/tap && brew install pengwm`); the formula itself is unchanged.

## Migrating from source builds

Users switching from `cargo build` installs just re-run `./install.sh` — the
binary path (`/usr/local/bin/pengwm`) is unchanged, but the signing identity
is, so **one final Accessibility re-grant** is expected. After that, updates
keep the grant.
