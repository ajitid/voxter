#!/usr/bin/env bash
set -euo pipefail

uuid="voxtral-speech-to-text@ajitid"
src_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/gnome-shell-extension/$uuid"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

gnome-extensions pack --force --out-dir "$tmp_dir" "$src_dir" >/dev/null
gnome-extensions install --force "$tmp_dir/$uuid.shell-extension.zip"

if gnome-extensions info "$uuid" >/dev/null 2>&1; then
  gnome-extensions enable "$uuid"
  cat <<'MSG'
Installed and enabled Voxtral GNOME Shell extension.
FIXME/Note: Re-installing does NOT reload the running extension in the current session. You still need to log out and log back in for changes to take effect. Check how others do the reload without re-login and fix.
Verify with: gnome-extensions info voxtral-speech-to-text@ajitid
MSG
else
  cat <<'MSG'
Installed Voxtral GNOME Shell extension files, but GNOME Shell has not loaded the new extension yet.
On GNOME Wayland, log out and log back in once, then run:
  gnome-extensions enable voxtral-speech-to-text@ajitid
Verify with:
  gnome-extensions info voxtral-speech-to-text@ajitid
MSG
fi
