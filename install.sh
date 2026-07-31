#!/usr/bin/env bash
set -euo pipefail

PREFIX="/usr/local/bin"
AGENT_LABEL="com.pengwm.daemon"
AGENT_PLIST="$HOME/Library/LaunchAgents/${AGENT_LABEL}.plist"
AGENT_LOG="$HOME/Library/Logs/pengwm.log"
USE_AGENT=1
UNINSTALL=0

usage() {
  cat <<'EOF'
PengWM install / update script

Usage:
  ./install.sh [options]

Options:
  --prefix DIR       Install binaries to DIR (default: /usr/local/bin)
  --no-agent         Do not install/load the launchd LaunchAgent
  --uninstall        Stop the daemon, remove the LaunchAgent and binaries
  --help             Show this help

The daemon is configured to start at login via a launchd LaunchAgent
($AGENT_LABEL). Re-running this script updates the binaries in place.
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
  echo "PengWM uninstalled."
}

install_binaries() {
  echo "Installing binaries to $PREFIX..."
  mkdir -p "$PREFIX"
  install -m 0755 "$SCRIPT_DIR/target/release/pengwm" "$PREFIX/pengwm"
  echo "Installed $PREFIX/pengwm"
  install -m 0755 "$SCRIPT_DIR/target/release/pengwm-bar" "$PREFIX/pengwm-bar"
  echo "Installed $PREFIX/pengwm-bar"
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

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: 'cargo' not found in PATH."
  echo "Install Rust via https://rustup.rs then re-run this script."
  exit 1
fi

echo "Building release binaries (this may take a while)..."
(cd "$SCRIPT_DIR" && cargo build --release)
install_binaries

if [[ "$USE_AGENT" == "1" ]]; then
  install_agent
else
  echo "Skipping LaunchAgent (--no-agent). Start the daemon manually:"
  echo "  $PREFIX/pengwm"
fi

print_next_steps
