# Plan: privileged Linux hotkey helper via pkexec

## Goal

Replace Linux `rdev::listen` in the main app with a small privileged helper launched via `pkexec`, similar to Show Me The Key. The helper reads raw Linux input events and streams minimal JSON-lines events to the main app over stdout.

Also rename the Cargo package/binary from `voxtral-speech-to-text` to `voxter`, while keeping the repository folder name unchanged.

## Decisions locked from user

- Helper name: `voxter-hotkey-helper`.
- Installed helper path: `/usr/local/libexec/voxter-hotkey-helper`.
- Linux hotkey: Right Alt / AltGr only.
- IPC: helper writes JSON-lines to stdout; main app reads child stdout.
- Prefer installed helper + polkit policy rather than dev-only `target/debug` path.

## Polkit auth persistence finding

Polkit supports temporary remembered authentication via `auth_self_keep` / `auth_admin_keep`.

References:

- `polkit(8)` docs: `auth_admin_keep` is like `auth_admin`, but authorization is kept for a brief period, e.g. five minutes.
- `polkit(8)` docs: if a rule returns `AUTH_SELF_KEEP` or `AUTH_ADMIN_KEEP`, future checks for the same action+subject can return `YES` for the next brief period.
- `pkexec(1)` docs: custom actions can be selected by `org.freedesktop.policykit.exec.path` annotation pointing to the full helper path.

Implications:

- We can avoid prompting on every app restart if restarts happen within the polkit keep window.
- Standard polkit does **not** provide an app-controlled, permanent “remember forever” permission in the `.policy` defaults.
- A sysadmin/user can install a `.rules` file returning `polkit.Result.YES` for this exact helper path/action to make it passwordless for selected users/groups, but that is a local admin policy decision.
- Plan should ship/install a `.policy` using `auth_admin_keep` by default, and document an optional `/etc/polkit-1/rules.d/` passwordless rule for trusted users.

## Current code context

- Main app entry: `src/main.rs`.
- Current hotkey listener: `spawn_hotkey_listener(proxy: EventLoopProxy<AppEvent>)` near the bottom of `src/main.rs`.
- Current Linux dependency: `rdev` with `wayland` feature in `Cargo.toml`.
- Current Linux startup text says Right Alt / AltGr.
- Current tray code is macOS-only.

## Design

### Binaries

Add explicit binary targets:

```toml
[[bin]]
name = "voxter"
path = "src/main.rs"

[[bin]]
name = "voxter-hotkey-helper"
path = "src/bin/voxter-hotkey-helper.rs"
```

Rename package:

```toml
[package]
name = "voxter"
```

### Helper behavior

Create `src/bin/voxter-hotkey-helper.rs`.

Responsibilities:

1. Verify it is running on Linux (`#[cfg(target_os = "linux")]`).
2. Listen to raw input events using `rdev::listen` with Wayland feature already enabled.
3. Emit one JSON object per line to stdout:

```json
{"event":"right_alt_press"}
{"event":"right_alt_release"}
{"event":"space_press"}
```

4. Flush stdout after every line.
5. Write diagnostics to stderr only.
6. Deduplicate held key repeats if `rdev` emits repeat press events while Right Alt is held.
7. Exit non-zero on listener initialization failure.

Hotkey mapping:

- Right Alt / AltGr press => `right_alt_press`
- Right Alt / AltGr release => `right_alt_release`
- Space press => `space_press`

Note: helper should not send text, audio, env vars, or any sensitive process state. It only emits the minimal events above.

### Main app Linux hotkey path

Refactor `spawn_hotkey_listener` into platform-specific implementations:

- macOS: keep current in-process `rdev::listen` implementation with `set_is_main_thread(false)`.
- Linux: spawn `/usr/bin/pkexec /usr/local/libexec/voxter-hotkey-helper`, read stdout lines, parse helper events, and forward to existing `ControlMsg`:
  - `right_alt_press` => `ControlMsg::SinglePress`
  - `right_alt_release` => `ControlMsg::StopHold`
  - `space_press` => `ControlMsg::SwitchToLatch`

Implementation notes:

- Use `std::process::Command` with `stdout(Stdio::piped())` and `stderr(Stdio::piped())` or inherit stderr.
- Prefer inheriting stderr initially so pkexec/helper errors are visible.
- Spawn a thread to read helper stdout with `BufRead::lines()`.
- If `pkexec` exits with code `126`, print a clear message: user cancelled authentication.
- If `pkexec` exits with code `127`, print a clear message: authentication failed or helper unavailable.
- If helper binary is missing, print install instructions.
- Do not fallback to in-process `rdev` on Linux; fail loudly so permission issues are visible.

### Polkit policy

Add `packaging/polkit/one.alynx?` no. Use our namespace:

Path in repo:

```text
packaging/polkit/com.ajitid.voxter.hotkey-helper.policy
```

Installed path:

```text
/usr/share/polkit-1/actions/com.ajitid.voxter.hotkey-helper.policy
```

Policy contents outline:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE policyconfig PUBLIC "-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN"
"http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd">
<policyconfig>
  <vendor>voxter</vendor>
  <vendor_url>https://github.com/ajitid/voxtral-speech-to-text</vendor_url>
  <action id="com.ajitid.voxter.hotkey-helper">
    <description>Run Voxter hotkey helper</description>
    <message>Authentication is required to let Voxter read keyboard hotkey events</message>
    <defaults>
      <allow_any>auth_admin</allow_any>
      <allow_inactive>auth_admin</allow_inactive>
      <allow_active>auth_admin_keep</allow_active>
    </defaults>
    <annotate key="org.freedesktop.policykit.exec.path">/usr/local/libexec/voxter-hotkey-helper</annotate>
  </action>
</policyconfig>
```

Double-check exact DTD string during implementation against local examples in `/usr/share/polkit-1/actions/*.policy`.

### Install script / docs

Add an install helper script, e.g.:

```text
scripts/install-linux-helper.sh
```

Script behavior:

1. `cargo build --release --bin voxter-hotkey-helper`
2. `sudo install -Dm755 target/release/voxter-hotkey-helper /usr/local/libexec/voxter-hotkey-helper`
3. `sudo install -Dm644 packaging/polkit/com.ajitid.voxter.hotkey-helper.policy /usr/share/polkit-1/actions/com.ajitid.voxter.hotkey-helper.policy`
4. Print reminder to restart/reload polkit if needed, though polkit monitors policy directories on many systems.

Update README or create `docs/linux-wayland.md` documenting:

- Why helper is needed on Wayland.
- It only emits hotkey press/release JSON.
- `auth_admin_keep` means password may be remembered briefly, commonly about five minutes.
- Optional local passwordless rule for trusted users:

```js
// /etc/polkit-1/rules.d/50-voxter-hotkey-helper.rules
polkit.addRule(function(action, subject) {
  if (action.id == "com.ajitid.voxter.hotkey-helper" && subject.active && subject.local && subject.isInGroup("wheel")) {
    return polkit.Result.YES;
  }
});
```

Mention group may be `sudo` instead of `wheel` on Debian/Ubuntu-family systems.

### Dependency cleanup

After moving Linux hotkey listening into helper:

- Keep `rdev` Linux dependency because helper uses it.
- Main Linux app no longer directly imports `rdev`, but the package-level Linux dependency is okay. If desired, split helper dependencies is not supported directly per-bin in Cargo, so keep target dependency.
- Keep macOS rdev dependency as-is.
- Keep Linux enigo Wayland dependency.

### Safety notes

- The helper runs as root and can read raw input events; keep it tiny and auditable.
- Do not add network, transcription, audio, config parsing, or broad command-line options to the helper.
- Do not accept arbitrary arguments that change behavior under retained authorization. If arguments are added later, avoid `auth_admin_keep` unless carefully validated. `pkexec(1)` warns retained/implicit authorization plus trusted user input can be a security hole.

## Implementation steps

1. Rename package to `voxter` and add `[[bin]]` entries in `Cargo.toml`.
2. Add `src/bin/voxter-hotkey-helper.rs` with Linux-only raw event listener.
3. Refactor `spawn_hotkey_listener` in `src/main.rs`:
   - `#[cfg(target_os = "macos")]` existing implementation.
   - `#[cfg(target_os = "linux")]` pkexec child/stdout JSON-lines implementation.
4. Add small helper-event parser in `src/main.rs` or a tiny shared module if desired.
5. Add polkit policy file under `packaging/polkit/`.
6. Add `scripts/install-linux-helper.sh`.
7. Update docs/README startup instructions and binary name.
8. Run:
   - `cargo fmt`
   - `cargo check --bin voxter`
   - `cargo check --bin voxter-hotkey-helper`
   - `cargo clippy --all-targets`
9. Install helper with script and manually test:
   - Run `cargo run --bin voxter`.
   - Confirm pkexec prompt appears first time.
   - Confirm Right Alt press starts recording and release stops recording.
   - Restart app within keep window and confirm prompt is skipped if polkit honors `auth_admin_keep` on this system.

## Open risks / checks during implementation

- `rdev::Key::AltGr` may or may not map exactly to the physical Right Alt on all layouts. If it fails, inspect helper debug output temporarily or switch helper to lower-level evdev/libinput key codes.
- `auth_admin_keep` duration and behavior can vary by polkit version/session/auth agent. Treat it as best-effort temporary caching, not a guaranteed permanent grant.
- KDE Wayland overlay positioning remains primary-monitor based; unrelated to helper.
