#!/usr/bin/env bash
set -euo pipefail

PENGWM_HOME="${PENGWM_HOME:-$HOME/.pengwm}"
BIN_DIR="$PENGWM_HOME/bin"
PREFIX="/usr/local/bin"
AGENT_LABEL="com.pengwm.daemon"
AGENT_PLIST="$HOME/Library/LaunchAgents/${AGENT_LABEL}.plist"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/pengwm"

usage() {
  cat <<EOF
PengWM uninstall script

Usage:
  ./uninstall.sh [options]

Options:
  --home DIR        Remove binaries from DIR/bin (default: \$PENGWM_HOME,
                    i.e. ~/.pengwm/bin)
  --prefix DIR      Remove CLI symlinks from DIR (default: /usr/local/bin)
  --keep-config     Do not remove ~/.config/pengwm
  --yes             Skip all confirmation prompts
  --help            Show this help
EOF
}

KEEP_CONFIG=0
ASSUME_YES=0

while [[ $# -gt 0 ]]; do
  case "$1" in
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

if [[ -d "$BIN_DIR" ]]; then
  for bin in pengwm pengwm-menubar; do
    if [[ -f "$BIN_DIR/$bin" ]] && confirm "Remove $BIN_DIR/$bin?"; then
      rm -f "$BIN_DIR/$bin"
      echo "Removed $BIN_DIR/$bin"
      REMOVED_SOMETHING=1
    fi
  done
  # Remove the bin dir (and the install root) when we emptied it.
  if [[ -d "$BIN_DIR" && -z "$(ls -A "$BIN_DIR" 2>/dev/null)" ]]; then
    rmdir "$BIN_DIR" 2>/dev/null || true
  fi
  if [[ -d "$PENGWM_HOME" && -z "$(ls -A "$PENGWM_HOME" 2>/dev/null)" ]]; then
    rmdir "$PENGWM_HOME" 2>/dev/null || true
  fi
fi

for bin in pengwm pengwm-menubar pengwm-bar; do
  # Only symlinks we made (pointing into the install root or a Cellar) are
  # ours to remove; a plain file could be someone's own copy.
  if [[ -L "$PREFIX/$bin" ]]; then
    rm -f "$PREFIX/$bin"
    echo "Removed symlink $PREFIX/$bin"
    REMOVED_SOMETHING=1
  fi
done

# Legacy .app layout (retired by ADR-0001) leftovers.
for appdir in "/Applications" "$HOME/Applications"; do
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

if [[ -d "$CONFIG_DIR" ]] && [[ "$KEEP_CONFIG" == "0" ]]; then
  if confirm "Remove configuration in $CONFIG_DIR?"; then
    rm -rf "$CONFIG_DIR"
    echo "Removed $CONFIG_DIR"
    REMOVED_SOMETHING=1
  else
    echo "Keeping $CONFIG_DIR"
  fi
fi

if [[ "$REMOVED_SOMETHING" == "1" || "$STOPPED_AGENT" == "1" ]]; then
  echo "PengWM uninstalled."
else
  echo "PengWM does not appear to be installed."
fi
