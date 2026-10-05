#!/usr/bin/env bash
set -euo pipefail

PREFIX="/usr/local/bin"
APP_DIRS=("/Applications" "$HOME/Applications")
AGENT_LABEL="com.pengwm.daemon"
AGENT_PLIST="$HOME/Library/LaunchAgents/${AGENT_LABEL}.plist"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/pengwm"

usage() {
  cat <<'EOF'
PengWM uninstall script

Usage:
  ./uninstall.sh [options]

Options:
  --app-dir DIR      Remove PengWM.app from DIR specifically
                     (default: try /Applications, then ~/Applications)
  --prefix DIR       Remove CLI shims from DIR (default: /usr/local/bin)
  --keep-config      Do not remove ~/.config/pengwm
  --yes              Skip all confirmation prompts
  --help             Show this help
EOF
}

KEEP_CONFIG=0
ASSUME_YES=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app-dir)
      if [[ $# -lt 2 ]]; then
        echo "error: --app-dir requires a directory argument"
        exit 1
      fi
      APP_DIRS=("$2")
      shift 2
      ;;
    --app-dir=*)
      APP_DIRS=("${1#*=}")
      shift
      ;;
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

REMOVED_SOMETHING=0

for appdir in "${APP_DIRS[@]}"; do
  if [[ -d "$appdir/PengWM.app" ]]; then
    if confirm "Remove $appdir/PengWM.app?"; then
      rm -rf "$appdir/PengWM.app"
      echo "Removed $appdir/PengWM.app"
      REMOVED_SOMETHING=1
    else
      echo "Keeping $appdir/PengWM.app"
    fi
  fi
done

for bin in pengwm pengwm-menubar pengwm-bar; do
  if [[ -L "$PREFIX/$bin" ]]; then
    rm -f "$PREFIX/$bin"
    echo "Removed shim $PREFIX/$bin"
    REMOVED_SOMETHING=1
  elif [[ -f "$PREFIX/$bin" ]]; then
    rm -f "$PREFIX/$bin"
    echo "Removed binary $PREFIX/$bin"
    REMOVED_SOMETHING=1
  fi
done

if [[ -d "$CONFIG_DIR" ]] && [[ "$KEEP_CONFIG" == "0" ]]; then
  if confirm "Remove configuration in $CONFIG_DIR?"; then
    rm -rf "$CONFIG_DIR"
    echo "Removed $CONFIG_DIR"
    REMOVED_SOMETHING=1
  else
    echo "Keeping $CONFIG_DIR"
  fi
fi

if [[ "$REMOVED_SOMETHING" == "1" ]]; then
  echo "PengWM uninstalled."
else
  echo "PengWM does not appear to be installed."
fi
