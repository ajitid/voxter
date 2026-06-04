# Plan: GNOME Wayland RemoteDesktop `UntilRevoked` persistence

## Goal

Make Linux/GNOME Wayland auto-typing request persistent XDG RemoteDesktop permission (`UntilRevoked`) now that Linux typing calls the portal directly instead of going through Enigo. Persist and rotate the portal `restore_token` across app restarts.

## User decisions

- Store the token in XDG state:
  - `$XDG_STATE_HOME/voxtral-speech-to-text/remote-desktop-restore-token`
  - fallback: `$HOME/.local/state/voxtral-speech-to-text/remote-desktop-restore-token`
- If token read/write fails, fail auto-typing immediately with an error. Do not silently fall back to prompting again.

## Research findings

### XDG RemoteDesktop portal behavior

Source: <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html>

`RemoteDesktop.SelectDevices` accepts:

- `restore_token` (`s`): token to restore a previous session.
- `persist_mode` (`u`):
  - `0`: do not persist
  - `1`: permissions persist while application is running
  - `2`: permissions persist until explicitly revoked

Important details from the spec:

- If persistence is granted, `RemoteDesktop.Start` returns a `restore_token` in its response.
- The restore token is single-use / invalidated after use.
- To restore next time, pass the newest token returned by the latest `Start` response.
- If restore fails because permissions were revoked or session details are no longer valid, the portal ignores the stale token and prompts normally.

### ashpd API mapping

Sources:

- `/home/ajit/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ashpd-0.13.11/src/desktop/mod.rs`
- `/home/ajit/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ashpd-0.13.11/src/desktop/remote_desktop.rs`

`ashpd` names the portal values as:

```rust
pub enum PersistMode {
    DoNot = 0,
    Application = 1,
    ExplicitlyRevoked = 2,
}
```

So the implementation should use `PersistMode::ExplicitlyRevoked` for the portal's `UntilRevoked` behavior.

`SelectDevicesOptions` already provides the methods needed:

```rust
.set_persist_mode(PersistMode::ExplicitlyRevoked)
.set_restore_token(token)
```

The `Start` response exposes:

```rust
response.restore_token()
```

## Current code points

- Linux restore token static currently named `ENIGO_RESTORE_TOKEN` at `src/main.rs` near the top.
- Linux typing is implemented in `try_type_linux()` in `src/main.rs` around the current `RemoteDesktop` code.
- Current Linux portal options use:

```rust
.set_persist_mode(PersistMode::Application)
```

- Current token storage is in-memory only.
- Documentation currently says cross-restart persistence is a future step in `docs/linux-workarounds.md`.

## Implementation plan

### 1. Rename Linux restore-token static

In `src/main.rs`, replace:

```rust
#[cfg(target_os = "linux")]
static ENIGO_RESTORE_TOKEN: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();
```

with:

```rust
#[cfg(target_os = "linux")]
static REMOTE_DESKTOP_RESTORE_TOKEN: std::sync::OnceLock<Mutex<Option<String>>> =
    std::sync::OnceLock::new();
```

Reason: Linux no longer uses Enigo for typing; the name should reflect XDG RemoteDesktop.

### 2. Add Linux-only token path helper

Add near the typing helpers in `src/main.rs`:

```rust
#[cfg(target_os = "linux")]
fn remote_desktop_restore_token_path() -> Result<std::path::PathBuf, String> {
    if let Some(state_home) = env::var_os("XDG_STATE_HOME") {
        return Ok(std::path::PathBuf::from(state_home)
            .join("voxtral-speech-to-text")
            .join("remote-desktop-restore-token"));
    }

    let home = env::var_os("HOME").ok_or_else(|| {
        "XDG_STATE_HOME is not set and HOME is not available; cannot locate RemoteDesktop restore token path"
            .to_string()
    })?;

    Ok(std::path::PathBuf::from(home)
        .join(".local")
        .join("state")
        .join("voxtral-speech-to-text")
        .join("remote-desktop-restore-token"))
}
```

No new dependency is required.

### 3. Add Linux-only token load helper

Add:

```rust
#[cfg(target_os = "linux")]
fn load_remote_desktop_restore_token() -> Result<Option<String>, String> {
    let path = remote_desktop_restore_token_path()?;
    match std::fs::read_to_string(&path) {
        Ok(token) => {
            let token = token.trim().to_string();
            if token.is_empty() {
                Ok(None)
            } else {
                Ok(Some(token))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!(
            "Failed to read RemoteDesktop restore token from {}: {e}",
            path.display()
        )),
    }
}
```

`NotFound` should be allowed because first run will have no token. Other read errors fail auto-typing per user choice.

### 4. Add Linux-only token save helper

Add:

```rust
#[cfg(target_os = "linux")]
fn save_remote_desktop_restore_token(token: &str) -> Result<(), String> {
    let path = remote_desktop_restore_token_path()?;
    let parent = path.parent().ok_or_else(|| {
        format!(
            "RemoteDesktop restore token path has no parent directory: {}",
            path.display()
        )
    })?;
    std::fs::create_dir_all(parent).map_err(|e| {
        format!(
            "Failed to create RemoteDesktop restore token directory {}: {e}",
            parent.display()
        )
    })?;
    std::fs::write(&path, format!("{token}\n")).map_err(|e| {
        format!(
            "Failed to write RemoteDesktop restore token to {}: {e}",
            path.display()
        )
    })?;
    Ok(())
}
```

Optional hardening if desired during implementation: on Unix, use `std::os::unix::fs::OpenOptionsExt` with mode `0o600` instead of `std::fs::write`. Since the token conveys portal permission, this is cleaner. If used, keep it Linux-only.

### 5. Initialize/read token before `SelectDevices`

In `try_type_linux()`, replace the current in-memory-only load block:

```rust
let restore_token = ENIGO_RESTORE_TOKEN
    .get_or_init(|| Mutex::new(None))
    .lock()
    .map_err(|_| "RemoteDesktop restore token lock poisoned".to_string())?
    .clone();
```

with logic that first checks memory, then disk, then writes the disk value back into memory:

```rust
let restore_token = {
    let token_cell = REMOTE_DESKTOP_RESTORE_TOKEN.get_or_init(|| Mutex::new(None));
    let mut guard = token_cell
        .lock()
        .map_err(|_| "RemoteDesktop restore token lock poisoned".to_string())?;

    if guard.is_none() {
        *guard = load_remote_desktop_restore_token()?;
    }

    guard.clone()
};
```

This avoids rereading the file every transcription once a token is loaded, but still supports cross-restart persistence.

### 6. Switch persist mode to `UntilRevoked`

In `try_type_linux()`, replace:

```rust
.set_persist_mode(PersistMode::Application)
```

with:

```rust
.set_persist_mode(PersistMode::ExplicitlyRevoked)
```

This maps to portal value `2`: persist until explicitly revoked.

### 7. Save rotated token after `Start`

Replace the current response token handling:

```rust
if let Some(token) = response.restore_token() {
    *ENIGO_RESTORE_TOKEN
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "RemoteDesktop restore token lock poisoned".to_string())? =
        Some(token.to_string());
}
```

with:

```rust
if let Some(token) = response.restore_token() {
    save_remote_desktop_restore_token(token)?;
    *REMOTE_DESKTOP_RESTORE_TOKEN
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "RemoteDesktop restore token lock poisoned".to_string())? =
        Some(token.to_string());
}
```

Because restore tokens may rotate, saving must happen after every successful `Start` response that returns a token.

### 8. Keep explicit session close behavior

Do not change the existing explicit:

```rust
session.close().await
```

The RemoteDesktop session should still be created only around typing and closed immediately after typing. `UntilRevoked` is about permission persistence, not keeping a live remote-control session open.

### 9. Update docs

Edit `docs/linux-workarounds.md` section `GNOME Remote Desktop prompt persistence`:

- Change current-state text from `Application`/memory-only to `ExplicitlyRevoked`/XDG state token persistence.
- Mention the storage path exactly:
  - `$XDG_STATE_HOME/voxtral-speech-to-text/remote-desktop-restore-token`
  - fallback `$HOME/.local/state/voxtral-speech-to-text/remote-desktop-restore-token`
- Mention that the token may rotate and the app overwrites the file with the latest returned token.
- Mention that deleting the file can force a fresh portal prompt, but true revocation should be done from GNOME's permission/privacy UI if available.
- Keep the note that the live RemoteDesktop session is still explicitly closed after typing, so the GNOME indicator should not stay active.

### 10. Update todo file

Update `.pi/todos/linux-gnome-latch-only.md` or create a new `.pi/todos/linux-remotedesktop-until-revoked.md` with checklist items:

- Add token path helper.
- Add load/save helpers.
- Switch `PersistMode` to `ExplicitlyRevoked`.
- Persist rotated restore token after `Start`.
- Rename restore-token static.
- Update docs.
- Verify.

Prefer creating a new todo file because this is a distinct follow-up.

## Verification plan

Run:

```bash
cargo fmt
cargo check
cargo clippy
```

Manual GNOME Wayland validation:

1. Delete existing token file, if any:
   ```bash
   rm -f "${XDG_STATE_HOME:-$HOME/.local/state}/voxtral-speech-to-text/remote-desktop-restore-token"
   ```
2. Run the app and trigger one transcription.
3. Approve GNOME's RemoteDesktop prompt with the persistent/remember option if shown.
4. Confirm the token file is created and non-empty.
5. Trigger another transcription in the same run; it should not prompt again.
6. Quit and restart the app.
7. Trigger another transcription; expected result is no prompt if GNOME granted persistent permission.
8. Confirm the GNOME remote-interaction indicator still disappears after typing finishes.

Failure cases to validate:

- Make the state directory unwritable and confirm auto-typing fails with a clear error instead of silently falling back.
- Put an invalid/stale token in the file and confirm GNOME prompts normally; after approval, the app should overwrite the file with the new token.

## Notes / risks

- GNOME/xdg-desktop-portal may still prompt if the user does not grant persistent permission or if permission is revoked from settings.
- The portal spec says restore tokens are invalidated after use, but some backends may return the same token. The app should still treat it as rotated and save the latest value every time.
- `ashpd` calls the enum variant `ExplicitlyRevoked`; docs/user-facing text may call it `UntilRevoked`.
- If a token is returned but saving it fails, the current plan fails the whole typing operation before sending keys if save occurs immediately after `Start`. This matches the user's "fail auto-typing on token I/O failure" choice. If preserving the current typing despite save failure is preferred later, move save after typing and report/log the error separately.
