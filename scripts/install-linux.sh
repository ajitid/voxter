#!/usr/bin/env bash
set -euo pipefail

APP_BIN_PATH="/usr/local/bin/voxter"
HELPER_PATH="/usr/local/libexec/voxter-hotkey-helper"
ON_SOUND_PATH="/usr/local/share/voxter/assets/on.mp3"
OFF_SOUND_PATH="/usr/local/share/voxter/assets/off.mp3"
VOXTER_SHARE_DIR="/usr/local/share/voxter"
ICON_PATH="/usr/share/icons/hicolor/scalable/status/voxter-symbolic.svg"
POLICY_PATH="/usr/share/polkit-1/actions/com.ajitid.voxter.hotkey-helper.policy"
RULES_PATH="/usr/share/polkit-1/rules.d/50-voxter-hotkey-helper.rules"

ACTION="install"
PARTS="helper,app-binary,app-assets"

usage() {
  cat <<'MSG'
Usage: scripts/install-linux.sh [--install|--uninstall] [--parts=helper,app-binary,app-assets]

Options:
  --install       Build and install selected Linux parts (default)
  --uninstall     Remove selected installed Linux parts
  --parts=LIST    Comma-separated parts to install/remove (default: all)
  -h, --help      Show this help

Parts:
  helper          voxter-hotkey-helper binary plus polkit policy/rules
  app-binary      main voxter binary
  app-assets      runtime sounds and hicolor voxter-symbolic tray icon
MSG
}

has_part() {
  local wanted="$1"
  local part
  IFS=',' read -ra parsed_parts <<<"$PARTS"
  for part in "${parsed_parts[@]}"; do
    [[ "$part" == "$wanted" ]] && return 0
  done
  return 1
}

normalize_parts() {
  local input="$1"
  local part
  local normalized=()
  local seen_helper=0
  local seen_app_binary=0
  local seen_app_assets=0

  if [[ -z "$input" ]]; then
    echo "--parts must not be empty" >&2
    exit 2
  fi

  IFS=',' read -ra parsed_parts <<<"$input"
  for part in "${parsed_parts[@]}"; do
    case "$part" in
      helper)
        if [[ $seen_helper -eq 0 ]]; then
          normalized+=("helper")
          seen_helper=1
        fi
        ;;
      app-binary)
        if [[ $seen_app_binary -eq 0 ]]; then
          normalized+=("app-binary")
          seen_app_binary=1
        fi
        ;;
      app-assets)
        if [[ $seen_app_assets -eq 0 ]]; then
          normalized+=("app-assets")
          seen_app_assets=1
        fi
        ;;
      "")
        echo "--parts contains an empty part" >&2
        exit 2
        ;;
      *)
        echo "Unknown Linux install part: $part" >&2
        usage >&2
        exit 2
        ;;
    esac
  done

  (IFS=','; echo "${normalized[*]}")
}

refresh_icon_caches() {
  local failed=0
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    if ! sudo gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor; then
      echo "Warning: gtk-update-icon-cache failed for /usr/share/icons/hicolor" >&2
      failed=1
    fi
  fi

  if command -v kbuildsycoca6 >/dev/null 2>&1; then
    if ! kbuildsycoca6 --noincremental >/dev/null 2>&1; then
      echo "Warning: kbuildsycoca6 cache refresh failed" >&2
      failed=1
    fi
  elif command -v kbuildsycoca5 >/dev/null 2>&1; then
    if ! kbuildsycoca5 --noincremental >/dev/null 2>&1; then
      echo "Warning: kbuildsycoca5 cache refresh failed" >&2
      failed=1
    fi
  fi

  return "$failed"
}

build_selected_binaries() {
  local cargo_args=(build --release)
  local need_build=0

  if has_part app-binary; then
    cargo_args+=(--bin voxter)
    need_build=1
  fi
  if has_part helper; then
    cargo_args+=(--bin voxter-hotkey-helper)
    need_build=1
  fi

  if [[ $need_build -eq 1 ]]; then
    cargo "${cargo_args[@]}"
  fi
}

install_selected() {
  cd "$(dirname "${BASH_SOURCE[0]}")/.."

  build_selected_binaries

  if has_part app-binary; then
    sudo install -Dm755 target/release/voxter "$APP_BIN_PATH"
  fi

  if has_part helper; then
    sudo install -Dm755 target/release/voxter-hotkey-helper "$HELPER_PATH"
    sudo install -Dm644 \
      packaging/polkit/com.ajitid.voxter.hotkey-helper.policy \
      "$POLICY_PATH"
    sudo install -Dm644 \
      packaging/polkit/50-voxter-hotkey-helper.rules \
      "$RULES_PATH"
  fi

  if has_part app-assets; then
    sudo install -Dm644 assets/on.mp3 "$ON_SOUND_PATH"
    sudo install -Dm644 assets/off.mp3 "$OFF_SOUND_PATH"
    sudo install -Dm644 assets/icons/hicolor/scalable/status/voxter-symbolic.svg \
      "$ICON_PATH"
    refresh_icon_caches || true
  fi

  echo "Installed selected Voxter Linux parts: $PARTS"
  if has_part helper; then
    cat <<MSG

Active local users in the 'wheel' group can run the helper without a password.
Other users fall back to auth_admin_keep from the policy file.
MSG
  fi
}

uninstall_selected() {
  if has_part app-binary; then
    sudo rm -f "$APP_BIN_PATH"
  fi

  if has_part helper; then
    sudo rm -f "$HELPER_PATH" "$POLICY_PATH" "$RULES_PATH"
  fi

  if has_part app-assets; then
    sudo rm -f "$ON_SOUND_PATH" "$OFF_SOUND_PATH" "$ICON_PATH"
    sudo rmdir "$VOXTER_SHARE_DIR/assets" "$VOXTER_SHARE_DIR" 2>/dev/null || true
    refresh_icon_caches || true
  fi

  echo "Removed selected Voxter Linux parts: $PARTS"
  if has_part helper; then
    echo "If you added a custom local override in /etc/polkit-1/rules.d, remove it manually."
  fi
}

for arg in "$@"; do
  case "$arg" in
    --install)
      ACTION="install"
      ;;
    --uninstall)
      ACTION="uninstall"
      ;;
    --parts=*)
      PARTS="${arg#--parts=}"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

PARTS="$(normalize_parts "$PARTS")"

case "$ACTION" in
  install)
    install_selected
    ;;
  uninstall)
    uninstall_selected
    ;;
  *)
    echo "Unknown action: $ACTION" >&2
    exit 2
    ;;
esac
