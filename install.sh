#!/usr/bin/env bash
set -euo pipefail

# PengWM install / update script
#
# Default: download a prebuilt, signed tarball from GitHub Releases.
#   ./install.sh
#   ./install.sh --version v0.5.0
#
# From source (old behavior, requires Rust):
#   ./install.sh --from-source

PREFIX="/usr/local/bin"
AGENT_LABEL="com.pengwm.daemon"
AGENT_PLIST="$HOME/Library/LaunchAgents/${AGENT_LABEL}.plist"
AGENT_LOG="$HOME/Library/Logs/pengwm.log"
REPO="Penqguin/PengWM"
VERSION="latest"
USE_AGENT=1
UNINSTALL=0
FROM_SOURCE=0

usage() {
  cat <<'EOF'
PengWM install / update script

Usage:
  ./install.sh [options]

Options:
  --version TAG        Release tag to install (default: latest).
                       Examples: --version v0.5.0, --version latest
  --from-source        Build from source with cargo instead of downloading
                       a prebuilt tarball (requires Rust).
  --repo OWNER/REPO    GitHub repo for releases (default: Penqguin/PengWM)
  --prefix DIR         Install binaries to DIR (default: /usr/local/bin)
  --no-agent           Do not install/load the launchd LaunchAgent
  --uninstall          Stop the daemon, remove the LaunchAgent and binaries
  --help               Show this help

The daemon is configured to start at login via a launchd LaunchAgent
($AGENT_LABEL). Re-running this script updates the binaries in place.
Prebuilt tarballs keep a stable code signature, so macOS Accessibility
grants survive updates — rebuilding from source re-prompts.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)
      if [[ $# -lt 2 ]]; then
        echo "error: --prefix requires a directory argument"
        exit 1
      fi
      PREFIX="$2"
      shift 2
      ;;
    --prefix=*)
      PREFIX="${1#*=}"
      shift
      ;;
    --version)
      VERSION="$2"
      shift 2
      ;;
    --version=*)
      VERSION="${1#*=}"
      shift
      ;;
    --repo)
      REPO="$2"
      shift 2
      ;;
    --repo=*)
      REPO="${1#*=}"
      shift
      ;;
    --from-source)
      FROM_SOURCE=1
      shift
      ;;
    --no-agent)
      USE_AGENT=0
      shift
      ;;
    --uninstall)
      UNINSTALL=1
      shift
      ;;
    --help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument '$1' (see --help)"
      exit 1
      ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

uninstall() {
  if [[ -f "$AGENT_PLIST" ]]; then
    echo "Unloading LaunchAgent..."
    launchctl bootout "gui/$(id -u)/$AGENT_LABEL" 2>/dev/null || launchctl unload "$AGENT_PLIST" 2>/dev/null || true
    rm -f "$AGENT_PLIST"
    echo "Removed $AGENT_PLIST"
  fi
  rm -f "$PREFIX/pengwm"
  echo "Removed $PREFIX/pengwm"
  rm -f "$PREFIX/pengwm-bar"
  echo "Removed $PREFIX/pengwm-bar"
  rm -f "$PREFIX/pengwm-menubar"
  echo "Removed $PREFIX/pengwm-menubar"
  echo "PengWM uninstalled."
}

arch_suffix() {
  case "$(uname -m)" in
    arm64)  echo "aarch64-apple-darwin" ;;
    x86_64) echo "x86_64-apple-darwin" ;;
    *)
      echo "error: unsupported architecture '$(uname -m)'" >&2
      exit 1
      ;;
  esac
}

resolve_version() {
  local want="$1"
  if [[ "$want" != "latest" ]]; then
    echo "$want"
    return
  fi
  # Query GitHub API for the latest release tag. Guarded so a failed request
  # (no releases yet, rate limit, offline) is handled below, not a set -e abort.
  curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
    | grep -m1 '"tag_name"' \
    | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/' \
    || true
}

install_from_source() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "error: 'cargo' not found in PATH."
    echo "Install Rust via https://rustup.rs then re-run, or drop --from-source to use a prebuilt tarball."
    exit 1
  fi
  echo "Building release binaries from source (this may take a while)..."
  echo "Note: source builds are ad-hoc signed — macOS will re-prompt for Accessibility."
  (cd "$SCRIPT_DIR" && cargo build --release)
  echo "Installing binaries to $PREFIX..."
  mkdir -p "$PREFIX"
  install -m 0755 "$SCRIPT_DIR/target/release/pengwm" "$PREFIX/pengwm"
  install -m 0755 "$SCRIPT_DIR/target/release/pengwm-bar" "$PREFIX/pengwm-bar"
  install -m 0755 "$SCRIPT_DIR/target/release/pengwm-menubar" "$PREFIX/pengwm-menubar"
  echo "Installed $PREFIX/pengwm, $PREFIX/pengwm-bar, $PREFIX/pengwm-menubar"
}

install_from_release() {
  local tag suffix url tmpdir tarball
  tag="$(resolve_version "$VERSION")"
  if [[ -z "$tag" ]]; then
    echo "error: could not resolve release version (is there a release at https://github.com/${REPO}/releases?)."
    echo "Pass --version vX.Y.Z explicitly, or use --from-source."
    exit 1
  fi
  suffix="$(arch_suffix)"
  tarball="pengwm-${tag}-${suffix}.tar.gz"
  url="https://github.com/${REPO}/releases/download/${tag}/${tarball}"
  echo "Downloading PengWM ${tag} (${suffix})..."
  echo "  ${url}"
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT
  # Download under the asset's original name so the .sha256 sidecar's
  # recorded filename matches for `shasum -a 256 -c`.
  curl -fsSL -o "$tmpdir/$tarball" "$url"
  # Verify checksum when the .sha256 sidecar exists (older releases may lack it).
  if curl -fsSL -o "$tmpdir/$tarball.sha256" "${url}.sha256" 2>/dev/null; then
    (cd "$tmpdir" && shasum -a 256 -c "$tarball.sha256")
    echo "Checksum OK."
  else
    echo "Warning: no checksum file found — skipping verification."
  fi
  tar xzf "$tmpdir/$tarball" -C "$tmpdir"
  for bin in pengwm pengwm-bar pengwm-menubar; do
    if [[ ! -f "$tmpdir/$bin" ]]; then
      echo "error: tarball is missing '$bin' (corrupt download?)"
      exit 1
    fi
  done
  # Verify the signature before installing: hard fail on tampering.
  if command -v codesign >/dev/null 2>&1; then
    for bin in pengwm pengwm-bar pengwm-menubar; do
      codesign --verify --verbose=1 "$tmpdir/$bin" || {
        echo "error: signature verification failed for $bin — refusing to install."
        exit 1
      }
    done
    echo "Signature OK (see: codesign -dv --verbose=4 $PREFIX/pengwm after install)."
    if spctl -a -t exec -vv "$tmpdir/pengwm" 2>&1 | grep -q "rejected"; then
      echo "Warning: Gatekeeper does not trust this build (likely ad-hoc signed, not notarized)."
      echo "It will still run, but macOS may re-prompt for Accessibility after updates."
    fi
  fi
  echo "Installing binaries to $PREFIX..."
  mkdir -p "$PREFIX"
  install -m 0755 "$tmpdir/pengwm" "$PREFIX/pengwm"
  install -m 0755 "$tmpdir/pengwm-bar" "$PREFIX/pengwm-bar"
  install -m 0755 "$tmpdir/pengwm-menubar" "$PREFIX/pengwm-menubar"
  echo "Installed $PREFIX/pengwm, $PREFIX/pengwm-bar, $PREFIX/pengwm-menubar (${tag})"
  rm -rf "$tmpdir"
  trap - EXIT
}

install_agent() {
  mkdir -p "$HOME/Library/LaunchAgents"
  mkdir -p "$HOME/Library/Logs"
  cat > "$AGENT_PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>$AGENT_LABEL</string>
	<key>ProgramArguments</key>
	<array>
		<string>$PREFIX/pengwm</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>StandardOutPath</key>
	<string>$AGENT_LOG</string>
	<key>StandardErrorPath</key>
	<string>$AGENT_LOG</string>
</dict>
</plist>
EOF
  echo "Wrote $AGENT_PLIST"

  launchctl bootout "gui/$(id -u)/$AGENT_LABEL" 2>/dev/null || launchctl unload "$AGENT_PLIST" 2>/dev/null || true
  launchctl bootstrap "gui/$(id -u)" "$AGENT_PLIST" 2>/dev/null || launchctl load "$AGENT_PLIST"
  echo "LaunchAgent loaded (daemon will start at login; starting now)"
}

print_next_steps() {
  echo
  echo "Next steps:"
  echo "  1. Grant Accessibility to PengWM:"
  echo "     System Settings -> Privacy & Security -> Accessibility"
  echo "     Add $PREFIX/pengwm"
  echo "     (Stable signed releases keep this grant across updates.)"
  echo "  2. Logs: $AGENT_LOG"
  echo "  3. Control it: pengwm focus left"
}

if [[ "$UNINSTALL" == "1" ]]; then
  uninstall
  exit 0
fi

if [[ "$(uname)" != "Darwin" ]]; then
  echo "error: PengWM is a macOS window manager and can only be installed on macOS."
  exit 1
fi

if [[ "$FROM_SOURCE" == "1" ]]; then
  install_from_source
else
  install_from_release
fi

if [[ "$USE_AGENT" == "1" ]]; then
  install_agent
else
  echo "Skipping LaunchAgent (--no-agent). Start the daemon manually:"
  echo "  $PREFIX/pengwm"
fi

print_next_steps
