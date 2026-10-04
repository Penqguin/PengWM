#!/usr/bin/env bash
# Build PengWM.app from the workspace's release binaries.
#
#   ./packaging/make_app.sh [bin-dir]
#
# bin-dir defaults to target/release. Produces ./PengWM.app in the repo root,
# signing it ad-hoc (or with $PENGWM_SIGN_IDENTITY when set, e.g. a Developer
# ID). CI calls this with the per-target release dir instead of signing loose
# binaries directly.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
bin_dir="${1:-$repo_root/target/release}"
app="$repo_root/PengWM.app"

for bin in pengwm pengwm-bar pengwm-menubar; do
  [[ -f "$bin_dir/$bin" ]] || { echo "error: missing $bin_dir/$bin — build first (cargo build --release)" >&2; exit 1; }
done

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$repo_root/packaging/PengWM.app/Contents/Info.plist" "$app/Contents/Info.plist"
cp "$repo_root/packaging/PengWM.icns" "$app/Contents/Resources/PengWM.icns"
for bin in pengwm pengwm-bar pengwm-menubar; do
  cp "$bin_dir/$bin" "$app/Contents/MacOS/$bin"
  chmod 0755 "$app/Contents/MacOS/$bin"
done

# Stamp the bundle version from the daemon's own --version output so the
# Info.plist never drifts from the crates' versions.
ver="$("$app/Contents/MacOS/pengwm" --version 2>/dev/null | awk '{print $NF}')"
if [[ "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  for key in CFBundleVersion CFBundleShortVersionString; do
    /usr/libexec/PlistBuddy -c "Set :$key $ver" "$app/Contents/Info.plist"
  done
fi

# Sign nested executables first, then the bundle (outermost last).
identity="${PENGWM_SIGN_IDENTITY:-}"
sign() {
  if [[ -n "$identity" ]]; then
    codesign --force --options runtime --timestamp --entitlements "$repo_root/packaging/entitlements.plist" -s "$identity" "$@"
  else
    codesign --force -s - "$@"
  fi
}
for bin in pengwm pengwm-bar pengwm-menubar; do
  sign "$app/Contents/MacOS/$bin"
done
sign "$app"
codesign --verify --verbose=1 "$app"

echo "Built $app"
