# Linux Wayland hotkey helper

On Linux, Voxter uses a small privileged helper for the global Left Control + Alt + Windows + H hotkey. Wayland does not generally allow regular desktop apps to read global keyboard events, so the main app launches the installed helper through `pkexec`.

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
- `/usr/share/polkit-1/rules.d/50-voxter-hotkey-helper.rules`

The helper is intentionally tiny. It reads raw input events and writes only these JSON-lines events to stdout:

```json
{"event":"hotkey_press"}
{"event":"hotkey_release"}
{"event":"space_press"}
```

It does not send audio, transcripts, environment variables, or other app state.

## Polkit authorization behavior

The installed rules file allows active local users in the `wheel` group to run the helper without a password:

```js
polkit.addRule(function(action, subject) {
  if (action.id === "com.ajitid.voxter.hotkey-helper" &&
      subject.isInGroup("wheel") && subject.local && subject.active) {
    return polkit.Result.YES;
  }
});
```

This matches Show Me The Key's model. On Debian/Ubuntu-family systems the admin group may be `sudo` instead of `wheel`; adjust the installed rule locally if needed.

The policy file still uses `auth_admin_keep` as a fallback for users who do not match the rules file. `auth_admin_keep` means a password may be remembered briefly, commonly around five minutes, but it is subject-based. With `pkexec`, restarting Voxter can create a new caller subject, so `auth_admin_keep` may still prompt on every app run even though one might expect it to remember compared to `auth_admin`.
