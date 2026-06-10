# Linux Wayland hotkey helper

On Linux, Voxter uses a small privileged helper for the global Right Alt / AltGr hotkey. Wayland does not generally allow regular desktop apps to read global keyboard events, so the main app launches the installed helper through `pkexec`.

Install it with:

```sh
scripts/install-linux-helper.sh
```

Uninstall it with:

```sh
scripts/install-linux-helper.sh --uninstall
```

This installs:

- `/usr/local/libexec/voxter-hotkey-helper`
- `/usr/share/polkit-1/actions/com.ajitid.voxter.hotkey-helper.policy`

The helper is intentionally tiny. It reads raw input events and writes only these JSON-lines events to stdout:

```json
{"event":"right_alt_press"}
{"event":"right_alt_release"}
{"event":"space_press"}
```

It does not send audio, transcripts, environment variables, or other app state.

The polkit policy uses `auth_admin_keep` for active sessions, so your password may be remembered briefly after approval, commonly around five minutes. This is best-effort polkit behavior and may vary by distro/session.

## Optional passwordless local rule

Trusted users can install a local polkit rule to skip prompts for this exact helper. For example:

```js
// /etc/polkit-1/rules.d/50-voxter-hotkey-helper.rules
polkit.addRule(function(action, subject) {
  if (action.id == "com.ajitid.voxter.hotkey-helper" && subject.active && subject.local && subject.isInGroup("wheel")) {
    return polkit.Result.YES;
  }
});
```

On Debian/Ubuntu-family systems the admin group may be `sudo` instead of `wheel`.
