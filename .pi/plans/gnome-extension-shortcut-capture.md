# Plan: switch GNOME Linux hotkey capture from XDG GlobalShortcuts portal to GNOME Shell extension

## Goal

Avoid intermittent GNOME/XDG portal `BindShortcuts` failures by removing the Linux runtime dependency on `org.freedesktop.portal.GlobalShortcuts` for recording hotkeys. On GNOME Wayland, the already-required GNOME Shell extension will own the recording shortcut and call the app over the existing app D-Bus service.

This intentionally does **not** add hold-to-record. The extension shortcut will trigger the existing Linux latch behavior: press once to start, press again to stop/process.

## Background / findings

Observed failure when repeatedly launching `cargo run`:

```text
Linux global shortcuts listener error: GlobalShortcuts binding was rejected or cancelled: Portal request didn't succeed with no information
```

Relevant findings:

- Current GNOME portal interface reports version 1:
  - `busctl --user introspect org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.GlobalShortcuts`
  - `version = 1`
- XDG docs for GlobalShortcuts v2 say `ListShortcuts` may return shortcuts bound in previous sessions by the same application.
- GNOME `xdg-desktop-portal-gnome` v1 implementation is effectively session-local for `ListShortcuts`; it returns the current `GlobalShortcutsSession.shortcuts`, populated only after `BindShortcuts` in that session.
- On this machine, dconf contains the stored binding:

```text
/org/gnome/settings-daemon/global-shortcuts/com.ajitid.VoxtralSpeechToText
shortcuts=[('vstt_record', {'shortcuts': <['<Super>c']>, 'description': <'Start/stop recording and transcribe'>})]
```

but portal `ListShortcuts` still returns `[]` for fresh sessions, causing repeated `BindShortcuts` attempts.

References:

- App portal code: `src/main.rs` around `spawn_hotkey_listener`, `run_linux_global_shortcuts_listener`, `run_linux_unbind_global_shortcuts`.
- App D-Bus service already used by extension: `src/main.rs` `LinuxAppActions` and `spawn_linux_app_dbus`.
- GNOME extension: `gnome-shell-extension/voxtral-speech-to-text@ajitid/extension.js`.
- Overlay D-Bus client: `src/ui/overlay/linux.rs`.
- XDG GlobalShortcuts docs: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html
- XDG Registry docs: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.host.portal.Registry.html
- GNOME extension keybinding example already installed locally: `/home/ajit/.local/share/gnome-shell/extensions/caffeine@patapon.info/extension.js` uses `Main.wm.addKeybinding(...)` with a schema key.

## Product decisions / assumptions

- Linux/GNOME uses the bundled GNOME Shell extension for recording shortcut capture.
- No hold mode for now; shortcut is latch/toggle.
- Breaking change is acceptable: remove the Linux `GlobalShortcuts` portal listener path instead of keeping it as fallback.
- Keep the GNOME extension requirement on Linux/GNOME. If extension is unavailable, app should continue to fail loudly as it already does for overlay.
- Keep the stable desktop file/app id work because the app still uses portals for RemoteDesktop typing and may need stable identity for permissions.
- Keep a legacy cleanup command for old GNOME portal bindings, but document it as legacy cleanup rather than normal setup.

## Implementation spec

### 1. Add app D-Bus method for shortcut activation

File: `src/main.rs`

Current interface:

```rust
#[zbus::interface(name = "com.ajitid.VoxtralSpeechToText.App1")]
impl LinuxAppActions {
    fn ping(&self) -> &str { ... }
    fn type_last_transcript(&self) { ... }
    fn quit(&self) { ... }
}
```

Add:

```rust
fn toggle_recording(&self) {
    let _ = self.proxy.send_event(AppEvent::Control(ControlMsg::SinglePress));
}
```

This maps the extension shortcut to the existing latch behavior.

Optional naming: use D-Bus method `ToggleRecording` generated from `toggle_recording` by zbus. Extension will call `ToggleRecording`.

### 2. Remove Linux XDG GlobalShortcuts listener from runtime

File: `src/main.rs`

Remove or cfg-disable the Linux-specific functions:

- `spawn_hotkey_listener(proxy: EventLoopProxy<AppEvent>)` for `#[cfg(target_os = "linux")]`
- `run_linux_global_shortcuts_listener(...)`

Then update `main()`:

Current flow:

```rust
#[cfg(target_os = "linux")]
let _linux_app_dbus = spawn_linux_app_dbus(proxy.clone())?;

spawn_hotkey_listener(proxy.clone());
```

Change to:

```rust
#[cfg(target_os = "linux")]
let _linux_app_dbus = spawn_linux_app_dbus(proxy.clone())?;

#[cfg(target_os = "macos")]
spawn_hotkey_listener(proxy.clone());
```

or provide a Linux no-op with a clear comment. Prefer explicit `#[cfg(target_os = "macos")]` call so there is no hidden Linux fallback.

Also update startup text:

```rust
#[cfg(target_os = "linux")]
println!("  LATCH: Press the GNOME Shell extension record shortcut to start recording; press it again to stop and transcribe");
```

Remove any temporary debug print such as:

```rust
println!("Portal existing global shortcuts: {existing_shortcuts:?}");
```

because portal code should be gone.

### 3. Reduce Linux dependencies/features

File: `Cargo.toml`

Current Linux `ashpd` features:

```toml
ashpd = { version = "0.13", default-features = false, features = ["async-io", "global_shortcuts", "remote_desktop", "screencast"] }
```

Remove `global_shortcuts` if no longer used:

```toml
ashpd = { version = "0.13", default-features = false, features = ["async-io", "remote_desktop", "screencast"] }
```

Run `cargo check`; if `screencast` is unused, do not remove it in this plan unless confirmed separately.

### 4. Add GNOME extension GSettings schema for the shortcut

Add file:

`gnome-shell-extension/voxtral-speech-to-text@ajitid/schemas/com.ajitid.VoxtralSpeechToText.Extension.gschema.xml`

Suggested schema:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<schemalist>
  <schema id="com.ajitid.VoxtralSpeechToText.Extension" path="/com/ajitid/VoxtralSpeechToText/Extension/">
    <key type="as" name="record-shortcut">
      <default><![CDATA[['<Super>c']]]></default>
      <summary>Record shortcut</summary>
      <description>Shortcut used by the Voxtral GNOME Shell extension to start/stop recording.</description>
    </key>
  </schema>
</schemalist>
```

Update `gnome-shell-extension/voxtral-speech-to-text@ajitid/metadata.json`:

Add:

```json
"settings-schema": "com.ajitid.VoxtralSpeechToText.Extension"
```

This lets the extension use `this.getSettings()`.

### 5. Register the keybinding in the GNOME extension

File: `gnome-shell-extension/voxtral-speech-to-text@ajitid/extension.js`

Add imports:

```js
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
```

Add constant:

```js
const RECORD_SHORTCUT = 'record-shortcut';
```

In `enable()`:

- initialize settings:

```js
this._settings = this.getSettings();
```

- register keybinding after the app watch is set up, or anywhere during enable:

```js
Main.wm.addKeybinding(
    RECORD_SHORTCUT,
    this._settings,
    Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
    Shell.ActionMode.ALL,
    () => this._callApp('ToggleRecording')
);
```

Use `Shell.ActionMode.ALL` so the shortcut works across normal/overview/lock-ish shell modes similarly to established extension examples. If `ALL` causes unwanted behavior during lock screen, narrow it to normal modes after testing.

In `disable()`:

```js
Main.wm.removeKeybinding(RECORD_SHORTCUT);
this._settings = null;
```

Place removal before destroying UI is fine. Ensure `removeKeybinding` is always called even if the app is unavailable.

The existing `_callApp(method)` helper already handles D-Bus calls to the app and logs failures. Reuse it.

### 6. Update GNOME extension install script to include schema

File: `scripts/install-gnome-shell-extension.sh`

Current pack command:

```bash
gnome-extensions pack --force --out-dir "$tmp_dir" "$src_dir" >/dev/null
```

Change to include the schema explicitly:

```bash
gnome-extensions pack \
  --force \
  --out-dir "$tmp_dir" \
  --schema "$src_dir/schemas/com.ajitid.VoxtralSpeechToText.Extension.gschema.xml" \
  "$src_dir" >/dev/null
```

Also validate `glib-compile-schemas` availability if needed. `gnome-extensions pack --schema` should include/compile the schema for the bundle; verify after install that `this.getSettings()` works. If not, add an explicit install-time `glib-compile-schemas "$installed_extension_dir/schemas"` after installation.

Keep `--uninstall` behavior unchanged.

### 7. Decide what to do with legacy portal unbind command

File: `src/main.rs`

Current `--unbind` clears GNOME portal shortcut dconf entries for `vstt_record`.

Recommended: keep it for one release as a legacy cleanup command, but change messages/docs:

- CLI behavior remains `cargo run -- --unbind`.
- Output says it clears legacy XDG/GNOME portal GlobalShortcuts bindings, not current extension shortcuts.
- Do not call or rely on this during normal setup.

Alternative clean breaking change: rename to `--clear-legacy-portal-shortcuts`. If doing this, update docs and reject `--unbind` with a clear error.

### 8. Update docs

Files:

- `docs/linux-hotkeys.md`
- `docs/linux-gnome-shell-overlay.md`
- `CLAUDE.md`

Required doc changes:

- Replace portal setup language with extension shortcut language.
- Remove “on first launch portal dialog” instructions from normal setup.
- Document default shortcut: `<Super>c`.
- Document how to change it via GSettings, since no prefs UI is planned:

```sh
gsettings set com.ajitid.VoxtralSpeechToText.Extension record-shortcut "['<Super>c']"
```

or e.g.:

```sh
gsettings set com.ajitid.VoxtralSpeechToText.Extension record-shortcut "['<Alt>space']"
```

- Mention `scripts/install-gnome-shell-extension.sh` installs the overlay, panel menu, and recording shortcut.
- Mark `cargo run -- --unbind` as legacy cleanup for old portal bindings only, if kept.
- Update CLAUDE Linux setup bullet to say GNOME extension handles shortcut capture; xdg portal GlobalShortcuts is intentionally not used on GNOME.

### 9. Verification

Run code checks:

```sh
cargo fmt
cargo check
cargo clippy
```

Run script checks:

```sh
bash -n scripts/install-gnome-shell-extension.sh
bash -n scripts/install-linux-desktop-file.sh
scripts/install-gnome-shell-extension.sh --help >/tmp/vstt-gnome-install-help.txt
scripts/install-linux-desktop-file.sh --help >/tmp/vstt-desktop-install-help.txt
```

Install/update extension:

```sh
scripts/install-gnome-shell-extension.sh
```

Then enable/check:

```sh
gnome-extensions enable voxtral-speech-to-text@ajitid
gnome-extensions info voxtral-speech-to-text@ajitid
gsettings get com.ajitid.VoxtralSpeechToText.Extension record-shortcut
```

If schema is not found, inspect installed extension dir and run/fix schema compilation:

```sh
find ~/.local/share/gnome-shell/extensions/voxtral-speech-to-text@ajitid -maxdepth 3 -type f -print
```

Run repeated launch test with terminalcp:

```sh
for i in 1 2 3 4 5 6 7 8 9 10; do
  name="vsttext$i"
  terminalcp start "$name" "cargo run"
  sleep 2.5
  terminalcp stdout "$name" 120
  terminalcp stop "$name" >/dev/null 2>&1 || true
  sleep 0.8
done
```

Expected:

- No `Linux global shortcuts listener error`.
- No `GlobalShortcuts binding was rejected...`.
- App starts and extension ping succeeds.

Manual functional test:

1. Run `cargo run`.
2. Press configured GNOME extension shortcut, default `<Super>c`.
3. Verify recording starts and overlay shows latch state.
4. Press shortcut again.
5. Verify recording stops and transcription starts/skips based on VAD.
6. Verify panel menu still works.

D-Bus smoke test for app method without pressing keys:

```sh
busctl --user call \
  com.ajitid.VoxtralSpeechToText.App \
  /com/ajitid/VoxtralSpeechToText/App \
  com.ajitid.VoxtralSpeechToText.App1 \
  ToggleRecording
```

Call twice to start then stop. Use only with caution because it records audio.

## Risks

- GNOME Shell extension APIs are GNOME-specific and can change across releases.
- `Main.wm.addKeybinding` needs a valid GSettings schema installed with the extension; install script must package schema correctly.
- Default `<Super>c` may conflict with a user/global shortcut. GNOME may refuse or ignore conflicting bindings. Users can change `record-shortcut` with `gsettings`.
- Existing portal dconf binding may remain but should no longer affect runtime once portal listener is removed.

## Rollback

If extension keybinding is unreliable:

- Restore the Linux portal listener functions and `ashpd` `global_shortcuts` feature from git.
- Reinstall desktop file and use `cargo run -- --unbind` / portal prompt flow as before.
