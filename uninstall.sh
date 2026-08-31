#!/usr/bin/env bash
set -euo pipefail

PREFIX="/usr/local/bin"
AGENT_LABEL="com.pengwm.daemon"
AGENT_PLIST="$HOME/Library/LaunchAgents/${AGENT_LABEL}.plist"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/pengwm"

usage() {
  cat <<'EOF'
PengWM uninstall script

Usage:
  ./uninstall.sh [options]

Options:
  --prefix DIR       Remove binaries from DIR (default: /usr/local/bin)
  --keep-config      Do not remove ~/.config/pengwm
  --yes              Skip all confirmation prompts
  --help             Show this help
EOF
}

KEEP_CONFIG=0
ASSUME_YES=0

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
    --keep-config)
      KEEP_CONFIG=1
      shift
      ;;
    --yes)
      ASSUME_YES=1
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

confirm() {
  [[ "$ASSUME_YES" == "1" ]] && return 0
  local prompt="$1"
  read -r -p "$prompt [y/N] " answer
  [[ "$answer" =~ ^[Yy]$ ]]
}

STOPPED_AGENT=0
if [[ -f "$AGENT_PLIST" ]]; then
  echo "Stopping the PengWM daemon…"
  launchctl bootout "gui/$(id -u)/$AGENT_LABEL" 2>/dev/null || launchctl unload "$AGENT_PLIST" 2>/dev/null || true
  rm -f "$AGENT_PLIST"
  STOPPED_AGENT=1
  echo "Removed $AGENT_PLIST"
fi

for bin in pengwm pengwm-bar pengwm-menubar; do
  if [[ -f "$PREFIX/$bin" ]]; then
    rm -f "$PREFIX/$bin"
    echo "Removed $PREFIX/$bin"
  fi
done

if [[ -d "$CONFIG_DIR" ]] && [[ "$KEEP_CONFIG" == "0" ]]; then
  if confirm "Remove configuration in $CONFIG_DIR?"; then
    rm -rf "$CONFIG_DIR"
    echo "Removed $CONFIG_DIR"
  else
    echo "Keeping $CONFIG_DIR"
  fi
fi

if [[ "$STOPPED_AGENT" == "1" ]] || [[ -f "$PREFIX/pengwm" ]] || [[ -f "$PREFIX/pengwm-bar" ]] || [[ -f "$PREFIX/pengwm-menubar" ]]; then
  echo "PengWM uninstalled."
else
  echo "PengWM does not appear to be installed."
fi
