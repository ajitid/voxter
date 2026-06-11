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
SERVICE_PATH="/etc/systemd/user/voxter.service"

ACTION="install"
PARTS="helper,app-binary,app-assets,app-service"

usage() {
  cat <<'MSG'
Usage: scripts/install-linux.sh [--install|--uninstall] [--parts=helper,app-binary,app-assets,app-service]

Options:
  --install       Build and install selected Linux parts (default)
  --uninstall     Remove selected installed Linux parts
  --parts=LIST    Comma-separated parts to install/remove (default: all)
  -h, --help      Show this help

Parts:
  helper          voxter-hotkey-helper binary plus polkit policy/rules
  app-binary      main voxter binary
  app-assets      runtime sounds and hicolor voxter-symbolic tray icon
  app-service     systemd user service file for starting voxter in the background
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
  local seen_app_service=0

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
      app-service)
        if [[ $seen_app_service -eq 0 ]]; then
          normalized+=("app-service")
          seen_app_service=1
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

xdg_config_home() {
  if [[ -n "${XDG_CONFIG_HOME:-}" && "$XDG_CONFIG_HOME" = /* ]]; then
    printf '%s\n' "$XDG_CONFIG_HOME"
  else
    printf '%s\n' "$HOME/.config"
  fi
}

prompt_and_store_user_settings() {
  local config_dir context_bias_path existing_api_key api_key context_bias prompt

  if [[ ! -t 0 ]]; then
    echo "Installing app-service requires an interactive terminal to enter VOXTER_API_KEY." >&2
    exit 1
  fi

  if ! command -v secret-tool >/dev/null 2>&1; then
    echo "Installing app-service requires secret-tool for Secret Service key storage." >&2
    echo "Install libsecret's secret-tool package for your distribution and rerun this script." >&2
    exit 1
  fi

  config_dir="$(xdg_config_home)/voxter"
  context_bias_path="$config_dir/context-bias"

  install -d -m 700 "$config_dir"

  if existing_api_key="$(secret-tool lookup application voxter key api-key 2>/dev/null)" && [[ -n "$existing_api_key" ]]; then
    prompt="Enter VOXTER_API_KEY (leave blank to keep existing Secret Service item): "
  else
    prompt="Enter VOXTER_API_KEY: "
  fi

  while true; do
    read -r -s -p "$prompt" api_key
    printf '\n'
    if [[ -n "$api_key" ]]; then
      printf '%s' "$api_key" | secret-tool store \
        --label="Voxter API key" \
        application voxter \
        key api-key
      break
    fi
    if [[ -n "${existing_api_key:-}" ]]; then
      break
    fi
    echo "VOXTER_API_KEY is required for the systemd user service."
  done

  read -r -p "Enter VOXTER_CONTEXT_BIAS (optional, leave blank to skip/keep existing): " context_bias
  if [[ -n "$context_bias" ]]; then
    printf '%s\n' "$context_bias" >"$context_bias_path"
    chmod 600 "$context_bias_path"
  fi

  echo "Stored Voxter API key in Secret Service."
  echo "Optional context bias path: $context_bias_path"
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

  if has_part app-service; then
    sudo install -Dm644 packaging/systemd/voxter.service "$SERVICE_PATH"
    systemctl --user daemon-reload || true
    prompt_and_store_user_settings
    systemctl --user enable --now voxter.service
  fi

  echo "Installed selected Voxter Linux parts: $PARTS"
  if has_part helper; then
    cat <<MSG

Active local users in the 'wheel' group can run the helper without a password.
Other users fall back to auth_admin_keep from the policy file.
MSG
  fi

  if has_part app-service; then
    cat <<'MSG'

The systemd user service has been enabled and started:
  systemctl --user enable --now voxter.service

Voxter will start automatically on login.

Voxter will read its API key from Secret Service.

Optional context bias is read from:
  ~/.config/voxter/context-bias
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

  if has_part app-service; then
    systemctl --user disable --now voxter.service || true
    sudo rm -f "$SERVICE_PATH"
    systemctl --user daemon-reload || true
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
