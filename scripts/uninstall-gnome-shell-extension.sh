#!/usr/bin/env bash
set -euo pipefail

uuid="voxtral-speech-to-text@ajitid"
dst_dir="$HOME/.local/share/gnome-shell/extensions/$uuid"

if gnome-extensions info "$uuid" >/dev/null 2>&1; then
  gnome-extensions disable "$uuid" 2>/dev/null || true
  gnome-extensions uninstall "$uuid" 2>/dev/null || true
fi

rm -rf "$dst_dir"

cat <<'MSG'
Uninstalled Voxtral GNOME Shell extension.
Verify removal with: gnome-extensions info voxtral-speech-to-text@ajitid
MSG
