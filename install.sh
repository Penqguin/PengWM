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
APP_DIR="/Applications"
AGENT_LABEL="com.pengwm.daemon"
AGENT_PLIST="$HOME/Library/LaunchAgents/${AGENT_LABEL}.plist"
AGENT_LOG="$HOME/Library/Logs/pengwm.log"
REPO="Penqguin/PengWM"
VERSION="latest"
USE_AGENT=1
UNINSTALL=0
FROM_SOURCE=0
EXPLICIT_PREFIX=0
NO_APP=0

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
  --app-dir DIR        Where to install PengWM.app (default: /Applications;
                       falls back to ~/Applications when not writable)
  --prefix DIR         Where to place the pengwm CLI symlink
                       (default: /usr/local/bin, falls back to ~/.local/bin)
  --no-app             Old flat layout: install loose binaries to --prefix
                       instead of a PengWM.app bundle
  --no-agent           Do not install/load the launchd LaunchAgent
  --uninstall          Stop the daemon, remove the LaunchAgent, .app and shims
  --help               Show this help

The app installs as /Applications/PengWM.app (appears in Launchpad) and the
daemon is configured to start at login via a launchd LaunchAgent
($AGENT_LABEL). Re-running this script updates the app in place.
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
      EXPLICIT_PREFIX=1
      shift 2
      ;;
    --prefix=*)
      PREFIX="${1#*=}"
      EXPLICIT_PREFIX=1
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
  --app-dir)
      if [[ $# -lt 2 ]]; then
        echo "error: --app-dir requires a directory argument"
        exit 1
      fi
      APP_DIR="$2"
      shift 2
      ;;
  --app-dir=*)
      APP_DIR="${1#*=}"
      shift
      ;;
  --no-app)
      NO_APP=1
      shift
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

# Pick a prefix that actually works without root. `curl ... | bash` has no
# way to elevate (no re-promptable password on a clean pipe), and a plain
# `./install.sh` run as the user can't write /usr/local either. Fall back to
# a user-owned prefix instead of dying at `install:` time.
if [[ "$EXPLICIT_PREFIX" == "1" ]]; then
  if mkdir -p "$PREFIX" 2>/dev/null && [[ -w "$PREFIX" ]]; then
    :
  else
    echo "error: --prefix '$PREFIX' is not writable. Re-run with sudo, or pick a user-owned directory (e.g. ~/.local/bin)."
    exit 1
  fi
else
  if ! mkdir -p "$PREFIX" 2>/dev/null || ! [[ -w "$PREFIX" ]]; then
    PREFIX="$HOME/.local/bin"
    mkdir -p "$PREFIX"
    echo "warning: default prefixes are not writable; installing to '$PREFIX' instead."
    echo "warning: make sure it is on your PATH:  export PATH=\"$PREFIX:\$PATH\""
  fi
fi

# Only meaningful when running the script from a checkout. When piped
# (`curl ... | bash`) there is no script file on disk, so BASH_SOURCE[0] is
# unset and `set -u` would abort on the plain `${BASH_SOURCE[0]}` reference.
# (-f is additionally checked by --from-source; a /dev/fd path from
# `bash <(curl ...)` is not a usable SCRIPT_DIR either way.)
SCRIPT_DIR=""
if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

uninstall() {
  if [[ -f "$AGENT_PLIST" ]]; then
    echo "Unloading LaunchAgent..."
    launchctl bootout "gui/$(id -u)/$AGENT_LABEL" 2>/dev/null || launchctl unload "$AGENT_PLIST" 2>/dev/null || true
    rm -f "$AGENT_PLIST"
    echo "Removed $AGENT_PLIST"
  fi
  # Match whichever dir the app actually landed in (default /Applications
  # falls back to ~/Applications at install time).
  local appdir=""
  if [[ -d "/Applications/PengWM.app" ]]; then
    appdir="/Applications"
  elif [[ -d "$HOME/Applications/PengWM.app" ]]; then
    appdir="$HOME/Applications"
  fi
  if [[ -n "$appdir" ]]; then
    rm -rf "$appdir/PengWM.app"
    echo "Removed $appdir/PengWM.app"
  fi
  for bin in pengwm pengwm-menubar pengwm-bar; do
    if [[ -L "$PREFIX/$bin" || -f "$PREFIX/$bin" ]]; then
      rm -f "$PREFIX/$bin"
      echo "Removed $PREFIX/$bin"
    fi
  done
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
  if [[ -z "$SCRIPT_DIR" || ! -f "$SCRIPT_DIR/Cargo.toml" ]]; then
    echo "error: --from-source needs the PengWM repo checkout (Cargo.toml + workspace), but this script was run outside one."
    echo "Use: git clone https://github.com/Penqguin/PengWM && cd PengWM && ./install.sh --from-source"
    exit 1
  fi
  echo "Building release binaries from source (this may take a while)..."
  echo "Note: source builds are ad-hoc signed — macOS will re-prompt for Accessibility."
  (cd "$SCRIPT_DIR" && cargo build --release)
  local bin_dir="$SCRIPT_DIR/target/release"
  if [[ "$NO_APP" == "1" ]]; then
    mkdir -p "$PREFIX"
    install -m 0755 "$bin_dir/pengwm" "$PREFIX/pengwm"
    install -m 0755 "$bin_dir/pengwm-menubar" "$PREFIX/pengwm-menubar"
    echo "Installed $PREFIX/pengwm, $PREFIX/pengwm-menubar"
  else
    bash "$SCRIPT_DIR/packaging/make_app.sh" "$bin_dir"
    install_app "$SCRIPT_DIR/PengWM.app"
  fi
}

# Copy a staged PengWM.app into APP_DIR and lay down the CLI symlinks so
# `pengwm ...` works from a shell without adding the bundle path to PATH.
install_app() {
  local staged="$1"
  if [[ ! -d "$staged" ]]; then
    echo "error: $staged is missing (corrupt download?)"
    exit 1
  fi
  mkdir -p "$APP_DIR" 2>/dev/null || true
  if [[ ! -w "$APP_DIR" ]]; then
    if [[ "$APP_DIR" == "/Applications" ]]; then
      APP_DIR="$HOME/Applications"
      mkdir -p "$APP_DIR"
      echo "warning: /Applications not writable; installing to '$APP_DIR'."
    else
      echo "error: --app-dir '$APP_DIR' is not writable. Re-run with sudo, or pick a user-owned directory (e.g. ~/Applications)."
      exit 1
    fi
  fi
  # ditto preserves code signatures and extended attributes — the other
  # copy tools silently strip what Gatekeeper checks.
  rm -rf "$APP_DIR/PengWM.app"
  ditto "$staged" "$APP_DIR/PengWM.app"
  # Re-register the bundle with LaunchServices. Replacing the bundle on disk
  # orphans the old registration; an unregistered bundle cannot be attributed
  # by TCC, so a launchd-spawned daemon fails AXIsProcessTrusted() even with
  # a fresh Accessibility grant in System Settings ("has access but never
  # works"). Manual `open` fixes it incidentally; lsregister makes it
  # deterministic.
  if [[ -x "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister" ]]; then
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister" -f "$APP_DIR/PengWM.app" 2>/dev/null \
      || echo "warning: lsregister failed — if Accessibility checks keep failing under launchd, run: open $APP_DIR/PengWM.app"
  fi
  echo "Installed $APP_DIR/PengWM.app"

  mkdir -p "$PREFIX" 2>/dev/null || true
  if [[ ! -w "$PREFIX" ]]; then
    if [[ "$EXPLICIT_PREFIX" == "1" ]]; then
      echo "error: --prefix '$PREFIX' is not writable. Re-run with sudo, or pick a user-owned directory (e.g. ~/.local/bin)."
      exit 1
    fi
    PREFIX="$HOME/.local/bin"
    mkdir -p "$PREFIX"
    echo "warning: '$PREFIX' not writable; installing CLI shims in '$PREFIX' instead."
    echo "warning: make sure it is on your PATH:  export PATH=\"$PREFIX:\$PATH\""
  fi
  for bin in pengwm pengwm-menubar; do
    ln -sf "$APP_DIR/PengWM.app/Contents/MacOS/$bin" "$PREFIX/$bin"
  done
  echo "CLI: $PREFIX/pengwm -> $APP_DIR/PengWM.app/Contents/MacOS/pengwm"
}

install_from_release() {
  local tag suffix url tmpdir tarball app_url app_tarball
  tag="$(resolve_version "$VERSION")"
  if [[ -z "$tag" ]]; then
    echo "error: could not resolve release version (is there a release at https://github.com/${REPO}/releases?)."
    echo "Pass --version vX.Y.Z explicitly, or use --from-source."
    exit 1
  fi
  suffix="$(arch_suffix)"
  # Primary: the PengWM.app bundle tarball. Older releases predate it —
  # fall back to the flat-binary tarball (the layout handled by --no-app).
  # Asset name is pengwm-app-* (not PengWM-*): GitHub matches release asset
  # URLs case-insensitively, so capital letters don't disambiguate.
  app_tarball="pengwm-app-${tag}-${suffix}.tar.gz"
  app_url="https://github.com/${REPO}/releases/download/${tag}/${app_tarball}"
  tarball="pengwm-${tag}-${suffix}.tar.gz"
  url="https://github.com/${REPO}/releases/download/${tag}/${tarball}"
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT

  # Download under the asset's original name so the .sha256 sidecar's
  # recorded filename matches for `shasum -a 256 -c`.
  local using_app=0
  if [[ "$NO_APP" != "1" ]] && curl -fsSL -o "$tmpdir/$app_tarball" "$app_url" 2>/dev/null; then
    using_app=1
    tarball="$app_tarball"
    url="$app_url"
    echo "Downloading PengWM ${tag} (${suffix}) — app bundle..."
  elif [[ "$NO_APP" != "1" ]]; then
    echo "Note: ${app_tarball} not published for ${tag} — falling back to the flat-binary layout."
    echo "      (Pin --version to a newer release, or keep the old layout with --no-app.)"
  fi
  echo "  ${url}"
  curl -fsSL -o "$tmpdir/$tarball" "$url"
  # Verify checksum when the .sha256 sidecar exists (older releases may lack it).
  if curl -fsSL -o "$tmpdir/$tarball.sha256" "${url}.sha256" 2>/dev/null; then
    (cd "$tmpdir" && shasum -a 256 -c "$tarball.sha256")
    echo "Checksum OK."
  else
    echo "Warning: no checksum file found — skipping verification."
  fi
  tar xzf "$tmpdir/$tarball" -C "$tmpdir"

  if [[ "$using_app" == "1" ]]; then
    if [[ ! -d "$tmpdir/PengWM.app" ]]; then
      echo "error: bundle tarball does not contain PengWM.app (corrupt download?)"
      exit 1
    fi
    if ! codesign --verify --verbose=1 "$tmpdir/PengWM.app" 2>/dev/null; then
      echo "error: signature verification failed for PengWM.app — refusing to install."
      exit 1
    fi
    echo "Signature OK (see: codesign -dv --verbose=4 $APP_DIR/PengWM.app after install)."
    if spctl -a -t exec -vv "$tmpdir/PengWM.app" 2>&1 | grep -q "rejected"; then
      echo "Warning: Gatekeeper does not trust this build (likely ad-hoc signed, not notarized)."
      echo "It will still run, but macOS may re-prompt for Accessibility after updates."
    fi
    install_app "$tmpdir/PengWM.app"
  elif [[ "$NO_APP" != "1" ]]; then
    NO_APP=1
  fi

  if [[ "$using_app" != "1" ]]; then
    # Flat layout: loose binaries from the legacy tarball. Releases older
    # than the bundle also shipped a `pengwm-bar` binary — it is tolerated
    # but no longer installed (the menubar is the only UI surface).
    for bin in pengwm pengwm-menubar; do
      if [[ ! -f "$tmpdir/$bin" ]]; then
        echo "error: tarball is missing '$bin' (corrupt download?)"
        exit 1
      fi
    done
    verify_signatures_loose "$tmpdir"
    echo "Installing binaries to $PREFIX..."
    mkdir -p "$PREFIX"
    install -m 0755 "$tmpdir/pengwm" "$PREFIX/pengwm"
    install -m 0755 "$tmpdir/pengwm-menubar" "$PREFIX/pengwm-menubar"
    echo "Installed $PREFIX/pengwm, $PREFIX/pengwm-menubar (${tag})"
  fi

  rm -rf "$tmpdir"
  trap - EXIT
}

verify_signatures_loose() {
  local dir="$1"
  if command -v codesign >/dev/null 2>&1; then
    for bin in pengwm pengwm-menubar; do
      codesign --verify --verbose=1 "$dir/$bin" || {
        echo "error: signature verification failed for $bin — refusing to install."
        exit 1
      }
    done
    echo "Signature OK (see: codesign -dv --verbose=4 $PREFIX/pengwm after install)."
    if spctl -a -t exec -vv "$dir/pengwm" 2>&1 | grep -q "rejected"; then
      echo "Warning: Gatekeeper does not trust this build (likely ad-hoc signed, not notarized)."
      echo "It will still run, but macOS may re-prompt for Accessibility after updates."
    fi
  fi
}

install_agent() {
  # In .app mode, launch the bundle executable directly (not the $PREFIX
  # symlink): keeps `current_exe()` sibling lookup honest so the daemon
  # finds pengwm-menubar next to itself in Contents/MacOS.
  local program="$PREFIX/pengwm"
  if [[ "$NO_APP" != "1" && -x "$APP_DIR/PengWM.app/Contents/MacOS/pengwm" ]]; then
    program="$APP_DIR/PengWM.app/Contents/MacOS/pengwm"
  fi
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
		<string>$program</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>StandardOutPath</key>
	<string>$AGENT_LOG</string>
	<key>StandardErrorPath</key>
	<string>$AGENT_LOG</string>
</dict>
</plist>
EOF
  echo "Wrote $AGENT_PLIST (program: $program)"

  launchctl bootout "gui/$(id -u)/$AGENT_LABEL" 2>/dev/null || launchctl unload "$AGENT_PLIST" 2>/dev/null || true
  launchctl bootstrap "gui/$(id -u)" "$AGENT_PLIST" 2>/dev/null || launchctl load "$AGENT_PLIST"
  echo "LaunchAgent loaded (daemon will start at login; starting now)"
}

print_next_steps() {
  echo
  echo "Next steps:"
  echo "  1. Grant Accessibility to PengWM:"
  echo "     System Settings -> Privacy & Security -> Accessibility"
  if [[ "$NO_APP" == "1" ]]; then
    echo "     Add $PREFIX/pengwm"
  else
    echo "     Add $APP_DIR/PengWM.app"
    echo "     (The app also appears in Launchpad; app updates keep the grant.)"
  fi
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
