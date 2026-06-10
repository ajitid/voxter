#!/usr/bin/env bash
set -euo pipefail

HELPER_PATH="/usr/local/libexec/voxter-hotkey-helper"
POLICY_PATH="/usr/share/polkit-1/actions/com.ajitid.voxter.hotkey-helper.policy"

usage() {
  cat <<'MSG'
Usage: scripts/install-linux-helper.sh [--install|--uninstall]

Options:
  --install     Build and install the Voxter Linux hotkey helper (default)
  --uninstall   Remove the installed helper and polkit policy
  -h, --help    Show this help
MSG
}

install_helper() {
  cd "$(dirname "${BASH_SOURCE[0]}")/.."

  cargo build --release --bin voxter-hotkey-helper

  sudo install -Dm755 target/release/voxter-hotkey-helper "$HELPER_PATH"
  sudo install -Dm644 \
    packaging/polkit/com.ajitid.voxter.hotkey-helper.policy \
    "$POLICY_PATH"

  cat <<MSG
Installed Voxter Linux hotkey helper and polkit policy:
  $HELPER_PATH
  $POLICY_PATH

Polkit usually notices new policy files automatically. If authorization does not work,
restart polkit or log out and back in, then run Voxter again.
MSG
}

uninstall_helper() {
  sudo rm -f "$HELPER_PATH" "$POLICY_PATH"

  cat <<MSG
Removed Voxter Linux hotkey helper and polkit policy:
  $HELPER_PATH
  $POLICY_PATH

If you added an optional passwordless local polkit rule, remove it manually, for example:
  /etc/polkit-1/rules.d/50-voxter-hotkey-helper.rules
MSG
}

case "${1:---install}" in
  --install)
    install_helper
    ;;
  --uninstall)
    uninstall_helper
    ;;
  -h|--help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
