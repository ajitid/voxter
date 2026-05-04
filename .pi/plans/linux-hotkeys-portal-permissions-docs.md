# Plan: replace Linux evdev raw-input permissions with XDG GlobalShortcuts portal docs + implementation

## Goal

Avoid recommending `sudo usermod -aG input "$USER"` for Linux hotkeys. Prefer a user-mediated, compositor/portal-managed global shortcut mechanism that does not grant every process owned by the user raw `/dev/input/event*` keyboard access.

## Web findings / references

- XDG Desktop Portal `org.freedesktop.portal.GlobalShortcuts` is designed for app global shortcuts and emits `Activated` / `Deactivated` regardless of focused window:
  - https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html
- The shortcut trigger format is defined by the freedesktop shortcuts spec; examples use xkbcommon names such as `CTRL+ALT+Return`:
  - https://specifications.freedesktop.org/shortcuts-spec/latest/
- Rust wrapper `ashpd` has a `global_shortcuts` feature and exposes `GlobalShortcuts::{create_session, bind_shortcuts, receive_activated, receive_deactivated}`:
  - https://docs.rs/ashpd/latest/ashpd/desktop/global_shortcuts/
  - https://docs.rs/ashpd/latest/ashpd/desktop/global_shortcuts/struct.GlobalShortcuts.html
- `udev TAG+=uaccess` is modern for many device classes, but ArchWiki explicitly warns not to set `uaccess` on input/block devices because unprivileged raw input remains sensitive:
  - https://wiki.archlinux.org/title/Udev
- systemd-logind/seat APIs are intended mainly for display servers/compositors to open and revoke device fds. They are not a good fit for an ordinary app that only needs a hotkey, especially while a Wayland compositor already controls the seat:
  - https://unix.stackexchange.com/questions/740428/how-where-does-udev-give-permission-to-x11-input-drivers-to-open-dev-input-even

## Design decision

Implement Linux hotkeys using **XDG Desktop Portal GlobalShortcuts** as a breaking replacement for direct `evdev`.

Rationale:

- It matches Wayland's security model.
- It avoids adding the user to `input` or installing a broad udev rule.
- It gives the compositor/desktop the opportunity to show a user consent/configuration dialog.
- It supports activation/deactivation signals, which can map to the app's HOLD mode.

Locked decision: do **not** force a preferred trigger. The current raw-evdev hotkey was **Right Alt alone**, but portal backends may reject modifier-only shortcuts or may not distinguish Right Alt/AltGr cleanly. Let the portal dialog/user choose the binding for the app's "Hold to record / release to transcribe" action.

## Scope

Files to change:

- `Cargo.toml`
- `src/main.rs`
- Remove `docs/linux-evdev-hotkeys.md`
- Add `docs/linux-hotkeys.md`
- `CLAUDE.md`

Optional files:

- `Cargo.lock` updated by `cargo`.

## Implementation phases

### Phase 1 — dependency setup

1. In `Cargo.toml`, replace the Linux-only `evdev` dependency:

   Current:

   ```toml
   [target.'cfg(target_os = "linux")'.dependencies]
   evdev = "0.13.2"
   ```

   Proposed:

   ```toml
   [target.'cfg(target_os = "linux")'.dependencies]
   ashpd = { version = "0.13", default-features = false, features = ["async-std", "global_shortcuts"] }
   futures-util = "0.3"
   ```

   If `ashpd` feature names differ at compile time, inspect `cargo add ashpd --features ...` / docs and adjust. `tokio` is also acceptable if `ashpd` requires it, but prefer the smallest runtime footprint for the single listener thread.

2. Run `cargo check` to let Cargo update `Cargo.lock` and surface exact API/features.

### Phase 2 — portal listener implementation

1. In `src/main.rs`, replace the Linux `spawn_hotkey_listener` / `run_linux_evdev_hotkey_listener` block around lines ~1191–1268.

2. Keep the public behavior at the `ControlMsg` layer, but map it to portal actions instead of raw key events:

   - `record` shortcut activated → `ControlMsg::SinglePress`
   - `record` shortcut deactivated → `ControlMsg::StopHold`
   - `switch_to_latch` shortcut activated → `ControlMsg::SwitchToLatch`
   - ignore `switch_to_latch` deactivation

   The portal dialog/user chooses bindings for both actions. This replaces the raw-evdev-specific "Space while holding Right Alt" behavior with a separate user-configured portal action.

3. Implementation sketch:

   ```rust
   #[cfg(target_os = "linux")]
   fn spawn_hotkey_listener(proxy: EventLoopProxy<AppEvent>) {
       thread::spawn(move || {
           if let Err(e) = async_std::task::block_on(run_linux_global_shortcuts_listener(proxy)) {
               eprintln!("Linux global shortcuts listener error: {e}");
               eprintln!("Hint: install/configure xdg-desktop-portal with GlobalShortcuts support, then bind the shortcut in the portal dialog.");
           }
       });
   }
   ```

4. Async listener sketch using `ashpd`:

   ```rust
   #[cfg(target_os = "linux")]
   async fn run_linux_global_shortcuts_listener(proxy: EventLoopProxy<AppEvent>) -> Result<(), String> {
       use ashpd::desktop::CreateSessionOptions;
       use ashpd::desktop::global_shortcuts::{
           BindShortcutsOptions, GlobalShortcuts, NewShortcut,
       };
       use futures_util::StreamExt;

       let portal = GlobalShortcuts::new().await.map_err(|e| e.to_string())?;
       if portal.version() < 1 {
           return Err("GlobalShortcuts portal is unavailable".to_string());
       }

       let session = portal
           .create_session(CreateSessionOptions::default())
           .await
           .map_err(|e| e.to_string())?;

       let shortcuts = [
           NewShortcut::new(
               "record",
               "Hold to record / release to transcribe",
               None, // no preferred trigger; let the portal dialog/user choose
           ),
           NewShortcut::new(
               "switch_to_latch",
               "Switch current hold recording to latch mode",
               None, // no preferred trigger; let the portal dialog/user choose
           ),
       ];

       let request = portal
           .bind_shortcuts(&session, &shortcuts, None, BindShortcutsOptions::default())
           .await
           .map_err(|e| e.to_string())?;
       let response = request.response().await.map_err(|e| e.to_string())?;
       if response.shortcuts().is_empty() {
           return Err("no global shortcut was bound".to_string());
       }

       let mut activated = portal.receive_activated().await.map_err(|e| e.to_string())?;
       let mut deactivated = portal.receive_deactivated().await.map_err(|e| e.to_string())?;

       loop {
           futures_util::select! {
               event = activated.next() => {
                   let Some(event) = event else { break; };
                   match event.shortcut_id() {
                       "record" => {
                           let _ = proxy.send_event(AppEvent::Control(ControlMsg::SinglePress));
                       }
                       "switch_to_latch" => {
                           let _ = proxy.send_event(AppEvent::Control(ControlMsg::SwitchToLatch));
                       }
                       _ => {}
                   }
               }
               event = deactivated.next() => {
                   let Some(event) = event else { break; };
                   if event.shortcut_id() == "record" {
                       let _ = proxy.send_event(AppEvent::Control(ControlMsg::StopHold));
                   }
               }
           }
       }

       Ok(())
   }
   ```

   Exact method names for `NewShortcut` / event accessors must be verified against installed `ashpd` docs/source during implementation.

5. If compile/API friction is high, fallback to direct `zbus` calls to `org.freedesktop.portal.Desktop` / `/org/freedesktop/portal/desktop` / `org.freedesktop.portal.GlobalShortcuts`; keep `ashpd` preferred for maintainability.

### Phase 3 — fallback policy

Locked decision: **portal-only, no evdev fallback**.

- If portal is unavailable, fail loudly with a clear message.
- Remove the Linux evdev implementation from code rather than hiding it behind an env var/feature.
- Do not silently fall back to raw `/dev/input/event*` because that reintroduces the exact permission/keylogging problem.

This is a breaking change for Linux users whose desktops do not support GlobalShortcuts. It is cleaner and safer.

### Phase 4 — docs updates

#### `docs/linux-hotkeys.md`

Create this new portal-first file and remove `docs/linux-evdev-hotkeys.md` entirely, because evdev is no longer kept in the implementation.

Content outline:

```md
# Linux hotkey permissions

The Linux backend uses the XDG Desktop Portal GlobalShortcuts interface. This does not require membership in the `input` group and does not grant raw `/dev/input/event*` access.

## Requirements

- A desktop/portal backend with `org.freedesktop.portal.GlobalShortcuts` support.
- `xdg-desktop-portal` and the desktop-specific backend installed/running.

## Setup

Run the app. On first launch, the desktop may show a shortcut binding/permission dialog. Bind the actions to whatever shortcuts you prefer:

- "Hold to record / release to transcribe"
- "Switch current hold recording to latch mode"

## Why not `input` group?

Adding your user to `input` lets any process running as your user read raw keyboard events, which is keylogging-capable. It is not recommended for normal desktop use.

## Unsupported portals

If your desktop/portal backend does not support `org.freedesktop.portal.GlobalShortcuts`, Linux hotkeys will fail loudly. There is intentionally no raw evdev fallback.
```

Delete the old `docs/linux-evdev-hotkeys.md` file.

#### `CLAUDE.md`

Replace Linux lines:

Current:

```md
- Linux/GNOME Wayland: `cargo run` uses evdev hotkeys and needs read access to `/dev/input/event*` keyboard devices
...
- On Linux: App needs evdev keyboard read permissions (for example, membership in the `input` group)
...
- Platform hotkey listener thread: `rdev` on macOS, `evdev` on Linux; sends `ControlMsg` to main thread
```

Proposed:

```md
- Linux/GNOME Wayland: `cargo run` uses XDG Desktop Portal GlobalShortcuts; no `input` group membership is required
...
- On Linux: App needs an `xdg-desktop-portal` backend with GlobalShortcuts support. Do not add users to the `input` group for this app.
...
- Platform hotkey listener thread: `rdev` on macOS, XDG Desktop Portal GlobalShortcuts on Linux; sends `ControlMsg` to main thread
```

Also update project overview if it still says Groq/Whisper while env/API mention Mistral Voxtral; this is unrelated but currently inconsistent.

### Phase 5 — verification

1. `cargo fmt`
2. `cargo check`
3. `cargo clippy`
4. Runtime checks on Linux session:

   ```sh
   busctl --user introspect org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.GlobalShortcuts
   ```

5. Run app:

   ```sh
   cargo run
   ```

   Verify:

   - Portal dialog appears on first bind, or existing binding is reused/listed.
   - Activating shortcut starts recording.
   - Releasing shortcut stops recording.
   - Failure mode is clear if portal unsupported.
   - User does not need to be in `input` group:

     ```sh
     id | grep -q '\binput\b' && echo "still in input group"
     ```

## Locked decisions from user

1. Linux hotkeys are a breaking portal-only replacement.
2. No evdev fallback in code.
3. Do not force a preferred trigger; let the portal dialog/user choose the binding.
4. Create `docs/linux-hotkeys.md` and remove `docs/linux-evdev-hotkeys.md`.
