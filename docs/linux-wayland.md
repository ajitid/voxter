# Linux Wayland support

On Linux/KDE, Voxter requires a working StatusNotifierItem system tray (for example, KDE Plasma's system tray). The tray menu provides:

- `Type last transcript`
- `Quit`

Startup is strict: if Voxter cannot find installed Linux assets or cannot register the StatusNotifierItem tray, it exits instead of continuing without a tray/icon/sounds.

## Linux install

On Linux, Voxter requires installed runtime assets and a small privileged helper for the global Left Control + Left Alt + Left Windows + J hotkey. Wayland does not generally allow regular desktop apps to read global keyboard events, so the main app launches the installed helper through `pkexec`.

Install Voxter's Linux binaries and assets with:

```sh
scripts/install-linux.sh
```

Uninstall them with:

```sh
scripts/install-linux.sh --uninstall
```

You can install or remove only selected parts with `--parts=helper,app-binary,app-assets`.

This installs:

- `/usr/local/bin/voxter`
- `/usr/local/libexec/voxter-hotkey-helper`
- `/usr/local/share/voxter/assets/on.mp3`
- `/usr/local/share/voxter/assets/off.mp3`
- `/usr/share/icons/hicolor/scalable/status/voxter-symbolic.svg`
- `/usr/share/polkit-1/actions/com.ajitid.voxter.hotkey-helper.policy`
- `/usr/share/polkit-1/rules.d/50-voxter-hotkey-helper.rules`

Linux startup is strict. If any required runtime asset is missing, Voxter exits with an error; rerun `scripts/install-linux.sh` to reinstall them.

The hotkey helper is intentionally tiny. It reads raw input events and writes only these JSON-lines events to stdout:

```json
{"event":"hotkey_press"}
{"event":"hotkey_release"}
```

It does not send audio, transcripts, environment variables, or other app state.

## Typing on Wayland

On Linux, Voxter types transcripts through the `eitype` Rust library, which uses libei via the XDG RemoteDesktop portal. This is intended to work on KDE/Wayland where Enigo/wtype-style virtual-keyboard approaches can fail.

Install/run requirements:

- `xdg-desktop-portal`
- `xdg-desktop-portal-kde` on KDE Plasma
- a running Wayland session with portal remote-control support

The first typing attempt may show a KDE remote-control/RemoteDesktop permission prompt. Approve it to allow Voxter to type into the active window. If the prompt offers an "Allow restoring on future sessions" option, enable it so the portal returns a restore token.

Voxter stores the eitype restore token at:

```text
${XDG_STATE_HOME:-$HOME/.local/state}/voxter/eitype-restore-token
```

`XDG_STATE_HOME` must be absolute when set. If it is unset, empty, or relative, Voxter uses `$HOME/.local/state`. Token persistence is strict: token read/write/path errors fail typing instead of silently falling back to a prompt-every-time flow.

Within a single Voxter run, Voxter keeps one eitype RemoteDesktop session open and reuses it for subsequent typing requests. This avoids starting a new portal session for every transcription, which reduces repeated KDE "Remote control session started" notifications. KDE may still show an active remote-control indicator while Voxter is running; quit Voxter to close the session.

Keyboard layout can be influenced with eitype/XKB environment variables such as `XKB_DEFAULT_LAYOUT`, `XKB_DEFAULT_VARIANT`, `XKB_DEFAULT_MODEL`, and `XKB_DEFAULT_OPTIONS`.

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
