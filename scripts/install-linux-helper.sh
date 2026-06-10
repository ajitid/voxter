#!/usr/bin/env bash
set -euo pipefail

HELPER_PATH="/usr/local/libexec/voxter-hotkey-helper"
POLICY_PATH="/usr/share/polkit-1/actions/com.ajitid.voxter.hotkey-helper.policy"
RULES_PATH="/usr/share/polkit-1/rules.d/50-voxter-hotkey-helper.rules"

usage() {
  cat <<'MSG'
Usage: scripts/install-linux-helper.sh [--install|--uninstall]

Options:
  --install     Build and install the Voxter Linux hotkey helper (default)
  --uninstall   Remove the installed helper, polkit policy, and wheel passwordless rule
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
  sudo install -Dm644 \
    packaging/polkit/50-voxter-hotkey-helper.rules \
    "$RULES_PATH"

  cat <<MSG
Installed Voxter Linux hotkey helper, polkit policy, and wheel passwordless rule:
  $HELPER_PATH
  $POLICY_PATH
  $RULES_PATH

Active local users in the 'wheel' group can run the helper without a password.
Other users fall back to auth_admin_keep from the policy file.

Polkit usually notices new policy/rules files automatically. If authorization does not work,
restart polkit or log out and back in, then run Voxter again.
MSG
}

uninstall_helper() {
  sudo rm -f "$HELPER_PATH" "$POLICY_PATH" "$RULES_PATH"

  cat <<MSG
Removed Voxter Linux hotkey helper, polkit policy, and wheel passwordless rule:
  $HELPER_PATH
  $POLICY_PATH
  $RULES_PATH

If you added a custom local override in /etc/polkit-1/rules.d, remove it manually.
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
