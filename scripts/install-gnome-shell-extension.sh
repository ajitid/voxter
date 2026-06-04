#!/usr/bin/env bash
set -euo pipefail

uuid="voxtral-speech-to-text@ajitid"
src_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/gnome-shell-extension/$uuid"
dst_dir="$HOME/.local/share/gnome-shell/extensions/$uuid"

rm -rf "$dst_dir"
mkdir -p "$(dirname "$dst_dir")"
cp -a "$src_dir" "$dst_dir"
gnome-extensions enable "$uuid" || true

cat <<'MSG'
Installed Voxtral GNOME Shell extension.
On Wayland, log out and log back in if GNOME Shell has not loaded the new extension yet.
Verify with: gnome-extensions info voxtral-speech-to-text@ajitid
MSG
