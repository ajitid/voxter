#!/usr/bin/env bash
set -euo pipefail

APP_ID="com.ajitid.VoxtralSpeechToText"
DESKTOP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
DESKTOP_FILE="$DESKTOP_DIR/${APP_ID}.desktop"

usage() {
  cat <<EOF_HELP
Usage: $0

Removes ~/.local/share/applications/${APP_ID}.desktop.
EOF_HELP
}

if [[ $# -gt 0 ]]; then
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
fi

if [[ -e "$DESKTOP_FILE" ]]; then
  rm "$DESKTOP_FILE"
  echo "Removed $DESKTOP_FILE"
else
  echo "Desktop file not installed: $DESKTOP_FILE"
fi

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
fi
