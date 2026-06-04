#!/usr/bin/env bash
set -euo pipefail

uuid="voxtral-speech-to-text@ajitid"
dst_dir="$HOME/.local/share/gnome-shell/extensions/$uuid"

gnome-extensions disable "$uuid" 2>/dev/null || true
rm -rf "$dst_dir"

cat <<'MSG'
Uninstalled Voxtral GNOME Shell extension.
On Wayland, log out and log back in if GNOME Shell has not unloaded the extension yet.
Verify removal with: gnome-extensions info voxtral-speech-to-text@ajitid
MSG
