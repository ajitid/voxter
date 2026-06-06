#!/usr/bin/env bash
set -euo pipefail

APP_ID="com.ajitid.VoxtralSpeechToText"
APP_NAME="Voxtral Speech to Text"
APP_PATH="target/debug/voxtral-speech-to-text"
UNINSTALL=false

usage() {
  cat <<EOF_HELP
Usage: $0 [--app-path PATH] [--uninstall]

Installs/updates ~/.local/share/applications/${APP_ID}.desktop.

Options:
  --app-path PATH  Path to the voxtral-speech-to-text binary.
                   Defaults to target/debug/voxtral-speech-to-text.
  --uninstall      Remove the installed desktop file instead of installing it.
  -h, --help       Show this help.
EOF_HELP
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app-path)
      [[ $# -ge 2 ]] || { echo "--app-path requires a value" >&2; exit 2; }
      APP_PATH="$2"
      shift 2
      ;;
    --uninstall)
      UNINSTALL=true
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

DESKTOP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
DESKTOP_FILE="$DESKTOP_DIR/${APP_ID}.desktop"

if [[ "$UNINSTALL" == true ]]; then
  if [[ -e "$DESKTOP_FILE" ]]; then
    rm "$DESKTOP_FILE"
    echo "Removed $DESKTOP_FILE"
  else
    echo "Desktop file not installed: $DESKTOP_FILE"
  fi

  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
  fi

  exit 0
fi

if [[ "$APP_PATH" != /* ]]; then
  APP_PATH="$(realpath -m "$APP_PATH")"
fi

if [[ ! -f "$APP_PATH" ]]; then
  cat >&2 <<EOF_ERR
Binary not found: $APP_PATH
Build it first, or pass --app-path:
  cargo build
  $0
  $0 --app-path /absolute/path/to/voxtral-speech-to-text
EOF_ERR
  exit 1
fi

if [[ ! -x "$APP_PATH" ]]; then
  echo "Binary is not executable: $APP_PATH" >&2
  exit 1
fi

mkdir -p "$DESKTOP_DIR"

cat > "$DESKTOP_FILE" <<EOF_DESKTOP
[Desktop Entry]
Type=Application
Name=${APP_NAME}
Comment=Record audio with a global shortcut and transcribe it with Mistral Voxtral
Exec=${APP_PATH}
Terminal=false
Categories=Utility;AudioVideo;Audio;Accessibility;
StartupNotify=false
EOF_DESKTOP

chmod 0644 "$DESKTOP_FILE"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
fi

echo "Installed $DESKTOP_FILE"
echo "Exec=$APP_PATH"
echo "Portal app id: $APP_ID"
