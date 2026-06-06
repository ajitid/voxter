#!/usr/bin/env bash
set -euo pipefail

uuid="voxtral-speech-to-text@ajitid"
repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src_dir="$repo_dir/gnome-shell-extension/$uuid"
dst_dir="$HOME/.local/share/gnome-shell/extensions/$uuid"
uninstall=false

usage() {
  cat <<EOF_HELP
Usage: $0 [--uninstall]

Installs/updates the Voxtral GNOME Shell extension.

Options:
  --uninstall  Remove the installed GNOME Shell extension instead of installing it.
  -h, --help   Show this help.
EOF_HELP
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --uninstall)
      uninstall=true
      shift
      ;;
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
done

if [[ "$uninstall" == true ]]; then
  if gnome-extensions info "$uuid" >/dev/null 2>&1; then
    gnome-extensions disable "$uuid" 2>/dev/null || true
    gnome-extensions uninstall "$uuid" 2>/dev/null || true
  fi

  rm -rf "$dst_dir"

  cat <<'MSG'
Uninstalled Voxtral GNOME Shell extension.
Verify removal with: gnome-extensions info voxtral-speech-to-text@ajitid
MSG
  exit 0
fi

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
