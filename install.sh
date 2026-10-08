#!/usr/bin/env bash
set -euo pipefail

# PengWM install / update script
#
# Default: download the prebuilt release tarball (two bare binaries +
# LICENSE) and install them into $PENGWM_HOME/bin (default ~/.pengwm/bin),
# with `pengwm`/`pengwm-menubar` symlinks on PATH and a launchd LaunchAgent
# that runs the installed daemon. Re-running this script updates in place.
#
# ---------------------------------------------------------------------------
# Signing note (ADR-0001): releases are AD-HOC SIGNED — macOS treats every
# update as a brand-new binary, so **every update costs one Accessibility
# re-grant** (a fresh re-prompt in System Settings). This is the standing
# trade-off of staying certificate-less; the same was true of the old
# PengWM.app bundle. A Developer ID certificate would remove this cost —
# see docs/distribution.md "Signing" for the ready-made checklist.
# ---------------------------------------------------------------------------
#
#   ./install.sh
#   ./install.sh --version v0.6.0
#
# From source (developers, requires Rust):
#   ./install.sh --from-source

PENGWM_HOME="${PENGWM_HOME:-$HOME/.pengwm}"
BIN_DIR="$PENGWM_HOME/bin"
PREFIX="/usr/local/bin"
AGENT_LABEL="com.pengwm.daemon"
AGENT_PLIST="$HOME/Library/LaunchAgents/${AGENT_LABEL}.plist"
AGENT_LOG="$HOME/Library/Logs/pengwm.log"
REPO="Penqguin/PengWM"
VERSION="latest"
USE_AGENT=1
UNINSTALL=0
FROM_SOURCE=0
EXPLICIT_PREFIX=0

usage() {
  cat <<EOF
PengWM install / update script

Usage:
  ./install.sh [options]

Options:
  --version TAG        Release tag to install (default: latest).
                       Examples: --version v0.6.0, --version latest
  --from-source        Build from source with cargo instead of downloading
                       a prebuilt tarball (developers; requires Rust).
  --repo OWNER/REPO    GitHub repo for releases (default: Penqguin/PengWM)
  --home DIR          Install root override (binaries in DIR/bin, env
                       PENGWM_HOME=DIR persisted to the LaunchAgent)
                       (default: ~/.pengwm)
  --prefix DIR         Where to place the pengwm CLI symlinks
                       (default: /usr/local/bin, falls back to ~/.local/bin)
  --no-agent           Do not install/load the launchd LaunchAgent
  --uninstall          Stop the daemon, remove the LaunchAgent, binaries, symlinks
  --help               Show this help

The binaries install to \$PENGWM_HOME/bin (default ~/.pengwm/bin) and the
daemon is configured to start at login via a launchd LaunchAgent
($AGENT_LABEL). Re-running this script updates the binaries in place.
Releases are ad-hoc signed: expect one Accessibility re-grant after every
update (see docs/adr/0001-bare-binary-distribution.md).
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
    --home)
      if [[ $# -lt 2 ]]; then
        echo "error: --home requires a directory argument"
        exit 1
      fi
      PENGWM_HOME="$2"
      BIN_DIR="$PENGWM_HOME/bin"
      shift 2
      ;;
    --home=*)
      PENGWM_HOME="${1#*=}"
      BIN_DIR="$PENGWM_HOME/bin"
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

# One-copy policy / migration: retire any leftover .app bundle from the
# v0.5.x layout. The bundle is replaced by the bare-binary layout
# (ADR-0001); a leftover bundle is a second installed copy and may shadow
# the new one. Removing it is expected — PENGWM_DEV=1 marks deliberate
# developer environments and keeps the bundle.
retire_old_bundle() {
  local bundle
  for bundle in "/Applications/PengWM.app" "$HOME/Applications/PengWM.app"; do
    [[ -d "$bundle" ]] || continue
    if [[ "${PENGWM_DEV:-0}" == "1" ]]; then
      echo "note: leaving $bundle in place (PENGWM_DEV=1)"
      continue
    fi
    echo "Removing old layout $bundle (replaced by the bare-binary install)..."
    launchctl bootout "gui/$(id -u)/$AGENT_LABEL" 2>/dev/null || launchctl unload "$AGENT_PLIST" 2>/dev/null || true
    rm -rf "$bundle"
  done
}

uninstall() {
  if [[ -f "$AGENT_PLIST" ]]; then
    echo "Unloading LaunchAgent..."
    launchctl bootout "gui/$(id -u)/$AGENT_LABEL" 2>/dev/null || launchctl unload "$AGENT_PLIST" 2>/dev/null || true
    rm -f "$AGENT_PLIST"
    echo "Removed $AGENT_PLIST"
  fi
  for bin in pengwm pengwm-menubar; do
    if [[ -L "$BIN_DIR/$bin" || -f "$BIN_DIR/$bin" ]]; then
      rm -f "$BIN_DIR/$bin"
      echo "Removed $BIN_DIR/$bin"
    fi
  done
  if [[ -d "$BIN_DIR" && -z "$(ls -A "$BIN_DIR" 2>/dev/null)" ]]; then
    rmdir "$BIN_DIR" 2>/dev/null || true
  fi
  for bin in pengwm pengwm-menubar pengwm-bar; do
    if [[ -L "$PREFIX/$bin" || -f "$PREFIX/$bin" ]]; then
      # Only remove symlinks pointing into PengWM's install root; a plain
      # file at $PREFIX (someone's own copy) is left alone unless it is
      # the symlink we made.
      if [[ -L "$PREFIX/$bin" ]]; then
        rm -f "$PREFIX/$bin"
        echo "Removed $PREFIX/$bin"
      fi
    fi
  done
  # Legacy layout leftovers.
  for appdir in "/Applications" "$HOME/Applications"; do
    if [[ -d "$appdir/PengWM.app" ]]; then
      rm -rf "$appdir/PengWM.app"
      echo "Removed $appdir/PengWM.app"
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

# One-copy policy: before anything on disk is replaced, look for PengWM
# copies other than the install target. Also report a daemon currently
# running from a path other than the target (typically started by hand or
# an IDE from a checkout): installing over it is safe, it restarts.
check_copies() {
  local strays=()
  local candidate toplevel pid ppid cmd
  # Old-layout bundles (retired by ADR-0001) are strays unless the script
  # is about to remove them; PENGWM_DEV=1 skips the warning for checkouts.
  for candidate in "/Applications/PengWM.app" "$HOME/Applications/PengWM.app"; do
    if [[ -d "$candidate" && "${PENGWM_DEV:-0}" != "1" ]]; then
      strays+=("$candidate")
    fi
  done
  if [[ ${#strays[@]} -gt 0 ]]; then
    echo "warning: found PengWM.app bundles (old layout, retired by ADR-0001):"
    for candidate in "${strays[@]}"; do
      echo "  $candidate"
    done
    echo "They will be removed by this install."
  fi

  # Daemons running from outside the bin dir. `pgrep -f` + a `ps -o
  # command=` cross-check on the exact daemon binary path filters out the
  # menubar sibling and unrelated processes. A launchd parent (pid 1)
  # means the daemon was started by the LaunchAgent and simply restarts on
  # update; anything else is worth a line.
  if command -v pgrep >/dev/null 2>&1; then
    while read -r pid; do
      if [[ -z "$pid" ]]; then
        continue
      fi
      cmd="$(ps -o command= -p "$pid" 2>/dev/null || true)"
      if [[ -z "$cmd" ]]; then
        continue
      fi
      # The daemon is a compiled binary that takes no arguments; match its
      # path end-anchored on the command line. That accepts interpreter
      # prefixes but not the pengwm-menubar sibling (different file name)
      # nor loose `pengwm focus left` invocations (they take arguments).
      case "$cmd" in
        "$BIN_DIR"/pengwm | "$BIN_DIR"/pengwm\ *) continue ;;
        */Cellar/pengwm/*/bin/pengwm | */Cellar/pengwm/*/bin/pengwm\ *) continue ;;
        */pengwm) ;;
        */pengwm\ *) ;;
        *) continue ;;
      esac
      ppid="$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
      if [[ -n "$ppid" && "$ppid" == "1" ]]; then
        continue
      fi
      echo "A pengwm daemon is currently running from $cmd (not the install target). Installing over the active daemon is safe — it restarts."
    done < <(pgrep -f pengwm 2>/dev/null || true)
  fi
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
  (cd "$SCRIPT_DIR" && cargo build --release)
  local bin_dir="$SCRIPT_DIR/target/release"
  install_binaries "$bin_dir"
}

# Install the two binaries into BIN_DIR and lay down the CLI symlinks so
# `pengwm ...` works from a shell without adding the bin dir to PATH.
install_binaries() {
  local source_dir="$1"
  for bin in pengwm pengwm-menubar; do
    if [[ ! -f "$source_dir/$bin" ]]; then
      echo "error: $source_dir/$bin is missing (corrupt download/build?)"
      exit 1
    fi
    # Hard gate: a broken/invalid signature must fail here, not ship.
    if ! codesign --verify --verbose=1 "$source_dir/$bin" 2>/dev/null; then
      echo "error: signature verification failed for $bin — refusing to install."
      exit 1
    fi
  done
  if spctl -a -t exec -vv "$source_dir/pengwm" 2>&1 | grep -q "rejected"; then
    echo "Warning: Gatekeeper does not trust this build (ad-hoc signed, not notarized)."
    echo "It will still run, but expect one Accessibility re-grant after every update."
  fi
  retire_old_bundle
  check_copies
  mkdir -p "$BIN_DIR"
  for bin in pengwm pengwm-menubar; do
    # cp, not ditto: loose binaries have no bundle to preserve; cp keeps
    # the code signature (signature data lives inside the Mach-O).
    cp -f "$source_dir/$bin" "$BIN_DIR/$bin"
    chmod 755 "$BIN_DIR/$bin"
  done
  echo "Installed binaries in $BIN_DIR"

  if [[ ! -w "$PREFIX" ]]; then
    if [[ "$EXPLICIT_PREFIX" == "1" ]]; then
      echo "error: --prefix '$PREFIX' is not writable. Re-run with sudo, or pick a user-owned directory (e.g. ~/.local/bin)."
      exit 1
    fi
    PREFIX="$HOME/.local/bin"
    mkdir -p "$PREFIX"
    echo "warning: '$PREFIX' not writable; installing CLI symlinks in '$PREFIX' instead."
    echo "warning: make sure it is on your PATH:  export PATH=\"$PREFIX:\$PATH\""
  fi
  for bin in pengwm pengwm-menubar; do
    ln -sf "$BIN_DIR/$bin" "$PREFIX/$bin"
  done
  echo "CLI: $PREFIX/pengwm -> $BIN_DIR/pengwm"
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
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT

  # Download under the asset's original name so the .sha256 sidecar's
  # recorded filename matches for `shasum -a 256 -c`.
  echo "Downloading PengWM ${tag} (${suffix})..."
  echo "  ${url}"
  if ! curl -fsSL -o "$tmpdir/$tarball" "$url" 2>/dev/null; then
    # A PengWM.app-only release predates the bare-binary switch-over
    # (v0.5.1–v0.5.x shipped the bundle; the flat tarball still exists for
    # those tags but installs an old layout).
    if curl -fsIL -o /dev/null "https://github.com/${REPO}/releases/download/${tag}/pengwm-app-${tag}-${suffix}.tar.gz" 2>/dev/null; then
      echo "error: ${tag} shipped the old PengWM.app bundle layout, which is no longer installed."
    else
      echo "error: ${tarball} not published for ${tag} (bad tag, or the release is still publishing)."
    fi
    echo "Pin --version to a newer release — see https://github.com/${REPO}/releases."
    exit 1
  fi
  # Verify checksum when the .sha256 sidecar exists (older releases may lack it).
  if curl -fsSL -o "$tmpdir/$tarball.sha256" "${url}.sha256" 2>/dev/null; then
    (cd "$tmpdir" && shasum -a 256 -c "$tarball.sha256")
    echo "Checksum OK."
  else
    echo "Warning: no checksum file found — skipping verification."
  fi
  tar xzf "$tmpdir/$tarball" -C "$tmpdir"
  install_binaries "$tmpdir"

  rm -rf "$tmpdir"
  trap - EXIT
}

install_agent() {
  # Launch the installed binary directly (not the $PREFIX symlink): keeps
  # `current_exe()` sibling lookup honest so the daemon finds
  # pengwm-menubar next to itself in the bin dir.
  local program="$BIN_DIR/pengwm"
  if [[ ! -x "$program" ]]; then
    echo "error: $program missing — install before configuring the LaunchAgent."
    exit 1
  fi
  mkdir -p "$HOME/Library/LaunchAgents"
  mkdir -p "$HOME/Library/Logs"
  # A custom install root must reach the daemon's one-copy policy: persist
  # it as an agent EnvironmentVariable so launchd-run daemons resolve the
  # same PENGWM_HOME install.sh used.
  local env_block=""
  if [[ "$PENGWM_HOME" != "$HOME/.pengwm" ]]; then
    env_block="	<key>EnvironmentVariables</key>
	<dict>
		<key>PENGWM_HOME</key>
		<string>$PENGWM_HOME</string>
	</dict>
"
  fi
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
${env_block}	<key>ProcessType</key>
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
  echo "     Add $BIN_DIR/pengwm"
  echo "     (Releases are ad-hoc signed: every update costs one re-grant"
  echo "      — see docs/adr/0001-bare-binary-distribution.md.)"
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
  echo "  $BIN_DIR/pengwm"
fi

print_next_steps
