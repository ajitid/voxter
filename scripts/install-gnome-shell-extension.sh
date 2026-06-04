#!/usr/bin/env bash
set -euo pipefail

uuid="voxtral-speech-to-text@ajitid"
src_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/gnome-shell-extension/$uuid"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

gnome-extensions pack --force --out-dir "$tmp_dir" "$src_dir" >/dev/null
gnome-extensions install --force "$tmp_dir/$uuid.shell-extension.zip"

if gnome-extensions info "$uuid" >/dev/null 2>&1; then
  # This refreshes extension state, but GNOME Shell 45+ uses ESM modules and
  # keeps loaded extension JS cached in the running shell process. On Wayland,
  # there is no supported non-destructive shell restart, so JS source changes
  # may still require logging out and back in.
  gnome-extensions disable --quiet "$uuid" >/dev/null 2>&1 || true
  gnome-extensions enable "$uuid"

  cat <<'MSG'
Installed Voxtral GNOME Shell extension files and toggled enable state.
Verify with:
  gnome-extensions info voxtral-speech-to-text@ajitid

Note: On GNOME Wayland, JavaScript code changes may still require logging out and back in.
MSG
else
  cat <<'MSG'
Installed Voxtral GNOME Shell extension files with overlay and panel menu, but GNOME Shell has not loaded the new extension yet.
On GNOME Wayland, log out and log back in once, then run:
  gnome-extensions enable voxtral-speech-to-text@ajitid
Verify with:
  gnome-extensions info voxtral-speech-to-text@ajitid
MSG
fi
