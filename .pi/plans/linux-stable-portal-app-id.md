# Plan: stable Linux portal app id + desktop file installer

## Goal

Make Linux `cargo run`/dev launches use a real, stable XDG/portal application identity instead of the transient `surface-transient` id, so XDG Desktop Portal `GlobalShortcuts.ListShortcuts` can find existing bindings and avoid unnecessary repeated `BindShortcuts` calls.

Chosen decisions:

- App id: `com.ajitid.VoxtralSpeechToText`
- Desktop file: `com.ajitid.VoxtralSpeechToText.desktop`
- Dev install: add a helper script that installs/updates the user desktop file. It defaults to `target/debug/voxtral-speech-to-text`, and accepts `--app-path <path>` for relative/absolute binary paths.
- Runtime strictness: on Linux, fail loudly if portal host-app registration fails; do not fall back to unregistered/transient behavior.

## References / rationale

- Current failure site: `src/main.rs:1563` (`run_linux_global_shortcuts_listener`) lists shortcuts, then calls `BindShortcuts` if `vstt_record` is absent.
- Current GNOME stored binding observed under transient app id:
  - `dconf dump /org/gnome/settings-daemon/global-shortcuts/` showed `[surface-transient]` with `vstt_record`.
- XDG GlobalShortcuts docs:
  - `BindShortcuts`: app can bind shortcuts for a session; this usually presents UI and can return an empty/subset result.
  - `ListShortcuts`: before `BindShortcuts`, it should return shortcuts successfully bound in a previous session by this application.
  - https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html
- XDG host portal Registry docs:
  - `Register(app_id, options)` associates an unsandboxed D-Bus peer with an application id.
  - The app id must match the basename of a `.desktop` file.
  - Must be done before any portal method call.
  - Registering is at most once.
  - https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.host.portal.Registry.html
- Desktop file naming convention:
  - Application desktop filename stem should be a valid D-Bus well-known name.
  - Reverse DNS in lowercase domain portion, then app name conventionally CamelCase.
  - Dashes are allowed but not recommended; they are not valid in related IDs like Flatpak app IDs.
  - `com.ajitid.VoxtralSpeechToText.desktop` is convention-conformant.
  - https://specifications.freedesktop.org/desktop-entry/latest/file-naming.html
- `ashpd` API available in the current dependency:
  - `ashpd::register_host_app(app_id: ashpd::AppID)` is exported by `ashpd-0.13.11/src/lib.rs`.
  - It uses the host portal Registry and skips only if sandboxed.

## Implementation patch spec

### 1. Add a Linux app-id constant in `src/main.rs`

Near the existing `REMOTE_DESKTOP_RESTORE_TOKEN` static (after it is fine), add:

```rust
#[cfg(target_os = "linux")]
const LINUX_APP_ID: &str = "com.ajitid.VoxtralSpeechToText";
```

This avoids duplicating the id in D-Bus names, portal registration, diagnostics, and future docs/logging.

### 2. Add Linux portal registration helper in `src/main.rs`

Add a new function before `spawn_linux_app_dbus` (near other Linux-specific helpers):

```rust
#[cfg(target_os = "linux")]
fn register_linux_portal_app_id() -> Result<(), String> {
    let app_id = ashpd::AppID::try_from(LINUX_APP_ID)
        .map_err(|e| format!("Invalid Linux app id {LINUX_APP_ID}: {e}"))?;

    pollster::block_on(ashpd::register_host_app(app_id)).map_err(|e| {
        format!(
            "Failed to register Linux portal app id {LINUX_APP_ID}: {e}\n\
Install the matching desktop file first, then retry:\n\
  cargo build\n\
  scripts/install-linux-desktop-file.sh\n\
The desktop filename must be {LINUX_APP_ID}.desktop and its basename must match the app id."
        )
    })?;

    println!("Registered Linux portal app id: {LINUX_APP_ID}");
    Ok(())
}
```

Notes:

- Use `ashpd::register_host_app`, not a private `ashpd::registry` module.
- It must be called before `GlobalShortcuts::new()`, `RemoteDesktop::new()`, or any other portal method.
- It is okay that `spawn_linux_app_dbus` uses a separate `zbus::blocking` connection for the app action bus name; that is not an XDG portal call.

### 3. Call portal registration early in `main`

In `main()`, after argument handling and before printing startup text / before any portal-using component can be spawned, add:

```rust
    #[cfg(target_os = "linux")]
    register_linux_portal_app_id()?;
```

Suggested exact location: immediately after the `--unbind` handling block and before:

```rust
    println!("Mistral Voxtral Speech-to-Text");
```

Reason: Registry docs require registration before any portal method call. Putting it this early makes that invariant obvious and avoids future accidental pre-registration portal calls.

### 4. Optionally reuse `LINUX_APP_ID` in D-Bus interface/name comments only; do not rename the existing app action D-Bus interface in this patch

Current D-Bus action interface/name:

- Interface: `com.ajitid.VoxtralSpeechToText.App1`
- Bus name: `com.ajitid.VoxtralSpeechToText.App`
- Object path: `/com/ajitid/VoxtralSpeechToText/App`

These are already compatible with the chosen app id. Avoid changing them unless a later cleanup wants constants for those too. The portal Registry app id does not require owning the exact same bus name.

### 5. Add `scripts/install-linux-desktop-file.sh`

Create executable file `scripts/install-linux-desktop-file.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

APP_ID="com.ajitid.VoxtralSpeechToText"
APP_NAME="Voxtral Speech to Text"
APP_PATH="target/debug/voxtral-speech-to-text"

usage() {
  cat <<EOF_HELP
Usage: $0 [--app-path PATH]

Installs/updates ~/.local/share/applications/${APP_ID}.desktop.

Options:
  --app-path PATH  Path to the voxtral-speech-to-text binary.
                   Defaults to target/debug/voxtral-speech-to-text.
  -h, --help       Show this help.
EOF_HELP
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app-path)
      [[ $# -ge 2 ]] || { echo "--app-path requires a value" >&2; exit 2; }
      APP_PATH="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$APP_PATH" != /* ]]; then
  APP_PATH="$(realpath -m "$APP_PATH")"
fi

if [[ ! -f "$APP_PATH" ]]; then
  cat >&2 <<EOF_ERR
Binary not found: $APP_PATH
Build it first, or pass --app-path:
  cargo build
  $0
  $0 --app-path /absolute/path/to/voxtral-speech-to-text
EOF_ERR
  exit 1
fi

if [[ ! -x "$APP_PATH" ]]; then
  echo "Binary is not executable: $APP_PATH" >&2
  exit 1
fi

DESKTOP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
DESKTOP_FILE="$DESKTOP_DIR/${APP_ID}.desktop"
mkdir -p "$DESKTOP_DIR"

cat > "$DESKTOP_FILE" <<EOF_DESKTOP
[Desktop Entry]
Type=Application
Name=${APP_NAME}
Comment=Record audio with a global shortcut and transcribe it with Mistral Voxtral
Exec=${APP_PATH}
Terminal=false
Categories=Utility;AudioVideo;Audio;Accessibility;
StartupNotify=false
EOF_DESKTOP

chmod 0644 "$DESKTOP_FILE"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
fi

echo "Installed $DESKTOP_FILE"
echo "Exec=$APP_PATH"
echo "Portal app id: $APP_ID"
```

Then `chmod +x scripts/install-linux-desktop-file.sh`.

Implementation note: `realpath -m` is GNU coreutils and appropriate for Linux/GNOME target. If portability is desired later, switch to Python/path canonicalization.

### 6. Update Linux hotkey docs

Edit `docs/linux-hotkeys.md`:

- In Requirements, add:
  - Matching installed desktop file: `com.ajitid.VoxtralSpeechToText.desktop`.
- In Setup, before “Run the app”, add a dev install step:

```md
For development runs, build the binary and install/update the user desktop file so portals can identify the app with a stable app id:

```sh
cargo build
scripts/install-linux-desktop-file.sh
```

By default the script points the desktop file to `target/debug/voxtral-speech-to-text`. To use another binary:

```sh
scripts/install-linux-desktop-file.sh --app-path ./target/release/voxtral-speech-to-text
```
```

- Mention that the app registers `com.ajitid.VoxtralSpeechToText` with the host portal on startup and intentionally fails if registration fails, because falling back to transient ids can cause repeated `BindShortcuts` prompts/rejections.

### 7. Update `CLAUDE.md`

In Environment Setup, add Linux note:

```md
- On Linux development runs, run `cargo build` then `scripts/install-linux-desktop-file.sh` before `cargo run`. The app registers the stable portal app id `com.ajitid.VoxtralSpeechToText` and fails loudly if the matching desktop file is missing or portal registration fails.
```

### 8. Consider but do not implement in this patch: state path migration

Current RemoteDesktop restore-token state path uses `voxtral-speech-to-text` under `$XDG_STATE_HOME`/`~/.local/state`. Do not change it in this patch. The app id stabilization should be narrowly scoped to portal identity and desktop file installation.

If a future packaging cleanup wants full XDG naming consistency, plan a separate migration for state/config paths.

## Verification plan

Run these after implementation:

1. Format/check:

```sh
cargo fmt
cargo check
```

2. Desktop file install:

```sh
cargo build
scripts/install-linux-desktop-file.sh
ls ~/.local/share/applications/com.ajitid.VoxtralSpeechToText.desktop
rg '^Exec=' ~/.local/share/applications/com.ajitid.VoxtralSpeechToText.desktop
```

3. Verify portal app id in D-Bus monitor:

```sh
terminalcp start gsmon "dbus-monitor --session \"interface='org.freedesktop.impl.portal.GlobalShortcuts'\""
terminalcp start vstt "cargo run"
sleep 2
terminalcp stream gsmon --since-last
terminalcp stdin vstt ::C-c
terminalcp stop vstt
terminalcp stop gsmon
```

Expected `CreateSession` backend call should include:

```text
string "com.ajitid.VoxtralSpeechToText"
```

not:

```text
string "surface-transient"
```

4. Run/cancel several times:

```sh
for i in 1 2 3 4 5; do
  terminalcp start "vstt$i" "cargo run"
  sleep 2
  terminalcp stream "vstt$i" --since-last | rg "Registered Linux portal app id|Using existing portal global shortcut binding|Linux global shortcuts listener error|BindShortcuts|rejected|cancelled|Waiting for hotkey" || true
  terminalcp stdin "vstt$i" ::C-c
  sleep 0.3
  terminalcp stop "vstt$i" >/dev/null 2>&1 || true
done
```

Expected:

- Startup prints `Registered Linux portal app id: com.ajitid.VoxtralSpeechToText`.
- After first successful binding under the new id, follow-up runs should print `Using existing portal global shortcut binding` and should not show the previous intermittent `GlobalShortcuts binding was rejected or cancelled`.

5. Inspect GNOME binding namespace:

```sh
dconf dump /org/gnome/settings-daemon/global-shortcuts/
```

Expected new section:

```text
[com.ajitid.VoxtralSpeechToText]
shortcuts=[('vstt_record', ...)]
```

Old `[surface-transient]` may remain until `cargo run -- --unbind` clears old entries by shortcut id.

## Risks / follow-ups

- First run after changing app id is effectively a new portal identity, so GNOME may prompt once to bind the shortcut again.
- If `ashpd::register_host_app` fails because registration happens after a portal call, the app should fail with the clear hint. This is intentional.
- If GNOME/portal validates the desktop file cache asynchronously, the install script's optional `update-desktop-database` may help, but a desktop/session cache delay is still possible. If observed, add docs to log out/in or restart `xdg-desktop-portal` as a troubleshooting step.
- `--unbind` currently finds apps by `vstt_record` and should clear both old and new namespaces. Keep that behavior.
