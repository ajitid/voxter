# Plan: Persist eitype RemoteDesktop restore token under XDG state

## Goal

Remember the Linux portal permission used by `eitype` so future Voxter launches can reuse the XDG RemoteDesktop restore token instead of prompting every time.

User decision locked:

- Token I/O policy: **strict**. If token path resolution, read, directory creation, file write, chmod, or rename fails, typing should fail with a clear error instead of silently falling back to non-persistent portal typing.

## References / findings

- Current Linux typing implementation: `src/main.rs:882-901` uses `EiType::connect_portal(EiTypeConfig::from_env())`, which discards restore tokens.
- `eitype 0.2.1` API in local crate source:
  - `~/.cargo/registry/src/index.crates.io-*/eitype-0.2.1/src/lib.rs:957-963`
  - `EiType::connect_portal_with_token(config, Option<&str>) -> Result<(EiType, Option<String>), EiTypeError>`
  - API docs comment says a valid restore token can skip the authorization dialog and a new token may be returned to save.
- `eitype` CLI currently stores its own token under cache (`~/.cache/eitype/restore_token`), but Voxter should use XDG state instead because this is persistent app state, not cache.
- Freedesktop XDG Base Directory Specification, Version 0.8:
  - `$XDG_STATE_HOME` is the base directory for user-specific state files.
  - If unset/empty, default is `$HOME/.local/state`.
  - State data persists between app restarts but is not important/portable enough for `$XDG_DATA_HOME`.
  - All XDG env var paths must be absolute; relative paths are invalid and should be ignored.
  - If creating a destination directory, create it with mode `0700`.
  - Source: https://specifications.freedesktop.org/basedir/latest/

## Token location

Use:

```text
${XDG_STATE_HOME:-$HOME/.local/state}/voxter/eitype-restore-token
```

Rules:

1. If `XDG_STATE_HOME` is set, non-empty, and absolute, use it.
2. If `XDG_STATE_HOME` is unset, empty, or relative, ignore it and use `$HOME/.local/state`.
3. If `$HOME` is unset, empty, or relative when fallback is needed, fail typing with a clear error.
4. Create the containing directory `.../voxter` with `0700` for newly-created directories.
5. Write the token file with `0600`.
6. Missing token file on first run is not an error; any other read error is an error.

## Patch spec

### 1. Add Linux-only token helpers in `src/main.rs`

Add these helper functions immediately after the macOS `try_type()` and before the Linux `try_type()`:

```rust
#[cfg(target_os = "linux")]
fn eitype_state_home() -> Result<std::path::PathBuf, String> {
    use std::path::PathBuf;

    if let Some(value) = env::var_os("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            return Ok(path);
        }
    }

    let home = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "Unable to resolve eitype restore-token path: HOME is unset".to_string())?;

    if !home.is_absolute() {
        return Err(format!(
            "Unable to resolve eitype restore-token path: HOME is not absolute: {}",
            home.display()
        ));
    }

    Ok(home.join(".local").join("state"))
}

#[cfg(target_os = "linux")]
fn eitype_restore_token_path() -> Result<std::path::PathBuf, String> {
    Ok(eitype_state_home()?
        .join("voxter")
        .join("eitype-restore-token"))
}

#[cfg(target_os = "linux")]
fn load_eitype_restore_token() -> Result<Option<String>, String> {
    use std::fs;
    use std::io::ErrorKind;

    let path = eitype_restore_token_path()?;
    match fs::read_to_string(&path) {
        Ok(token) => {
            let token = token.trim().to_string();
            if token.is_empty() {
                Ok(None)
            } else {
                Ok(Some(token))
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Failed to read eitype restore token from {}: {error}",
            path.display()
        )),
    }
}

#[cfg(target_os = "linux")]
fn save_eitype_restore_token(token: &str) -> Result<(), String> {
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

    let path = eitype_restore_token_path()?;
    let parent = path.parent().ok_or_else(|| {
        format!(
            "Failed to resolve parent directory for eitype restore token path: {}",
            path.display()
        )
    })?;

    let mut dir_builder = fs::DirBuilder::new();
    dir_builder.recursive(true).mode(0o700);
    dir_builder.create(parent).map_err(|error| {
        format!(
            "Failed to create eitype restore-token directory {}: {error}",
            parent.display()
        )
    })?;

    let temp_path = path.with_extension("tmp");
    match fs::remove_file(&temp_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Failed to remove stale eitype restore-token temp file {}: {error}",
                temp_path.display()
            ));
        }
    }

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp_path)
        .map_err(|error| {
            format!(
                "Failed to create eitype restore-token temp file {}: {error}",
                temp_path.display()
            )
        })?;

    file.write_all(token.as_bytes()).map_err(|error| {
        format!(
            "Failed to write eitype restore token to {}: {error}",
            temp_path.display()
        )
    })?;
    file.write_all(b"\n").map_err(|error| {
        format!(
            "Failed to write eitype restore token newline to {}: {error}",
            temp_path.display()
        )
    })?;
    file.sync_all().map_err(|error| {
        format!(
            "Failed to sync eitype restore-token temp file {}: {error}",
            temp_path.display()
        )
    })?;
    drop(file);

    fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        format!(
            "Failed to set eitype restore-token temp file permissions on {}: {error}",
            temp_path.display()
        )
    })?;

    fs::rename(&temp_path, &path).map_err(|error| {
        format!(
            "Failed to move eitype restore-token temp file {} to {}: {error}",
            temp_path.display(),
            path.display()
        )
    })?;

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        format!(
            "Failed to set eitype restore-token file permissions on {}: {error}",
            path.display()
        )
    })?;

    Ok(())
}
```

Notes:

- Use fully-qualified `std::path::PathBuf` in signatures to avoid top-level import churn.
- `env` is already imported at the top of `src/main.rs` (`use std::env;`).
- `fs::DirBuilder` with `DirBuilderExt::mode(0o700)` applies to newly-created directories. Do not chmod existing directories.
- The temp-file + rename pattern avoids leaving a partially-written token at the final path.

### 2. Replace Linux `try_type()` in `src/main.rs`

Replace the current Linux implementation:

```rust
#[cfg(target_os = "linux")]
fn try_type(text: &str) -> Result<(), String> {
    use eitype::{EiType, EiTypeConfig};

    let mut typer = EiType::connect_portal(EiTypeConfig::from_env()).map_err(|e| {
        format!(
            "eitype portal connection error: {e}. \
             Ensure xdg-desktop-portal and xdg-desktop-portal-kde are installed/running, \
             and approve the remote-control prompt if shown."
        )
    })?;

    let result = typer
        .type_text(text)
        .map_err(|e| format!("eitype text error: {e}"));
    typer.close();
    result
}
```

With:

```rust
#[cfg(target_os = "linux")]
fn try_type(text: &str) -> Result<(), String> {
    use eitype::{EiType, EiTypeConfig};

    let saved_token = load_eitype_restore_token()?;
    let (mut typer, new_token) =
        EiType::connect_portal_with_token(EiTypeConfig::from_env(), saved_token.as_deref())
            .map_err(|e| {
                format!(
                    "eitype portal connection error: {e}. \
                     Ensure xdg-desktop-portal and xdg-desktop-portal-kde are installed/running, \
                     and approve the remote-control prompt if shown."
                )
            })?;

    if let Some(token) = new_token.as_deref() {
        if let Err(error) = save_eitype_restore_token(token) {
            typer.close();
            return Err(error);
        }
    }

    let result = typer
        .type_text(text)
        .map_err(|e| format!("eitype text error: {e}"));
    typer.close();
    result
}
```

Rationale:

- Load token before connecting; strict read/path errors stop typing.
- Pass token to `connect_portal_with_token()`.
- If the portal returns a token, save it before typing. Strict save errors close the portal connection and fail typing.
- Preserve explicit `close()` on all post-connect error paths.

### 3. Update `docs/linux-wayland.md`

In the `## Typing on Wayland` section, replace:

```md
The first typing attempt may show a KDE remote-control/RemoteDesktop permission prompt. Approve it to allow Voxter to type into the active window.

Keyboard layout can be influenced with eitype/XKB environment variables such as `XKB_DEFAULT_LAYOUT`, `XKB_DEFAULT_VARIANT`, `XKB_DEFAULT_MODEL`, and `XKB_DEFAULT_OPTIONS`.
```

With:

```md
The first typing attempt may show a KDE remote-control/RemoteDesktop permission prompt. Approve it to allow Voxter to type into the active window. If the prompt offers an "Allow restoring on future sessions" option, enable it so the portal returns a restore token.

Voxter stores the eitype restore token at:

```text
${XDG_STATE_HOME:-$HOME/.local/state}/voxter/eitype-restore-token
```

`XDG_STATE_HOME` must be absolute when set. If it is unset, empty, or relative, Voxter uses `$HOME/.local/state`. Token persistence is strict: token read/write/path errors fail typing instead of silently falling back to a prompt-every-time flow.

Keyboard layout can be influenced with eitype/XKB environment variables such as `XKB_DEFAULT_LAYOUT`, `XKB_DEFAULT_VARIANT`, `XKB_DEFAULT_MODEL`, and `XKB_DEFAULT_OPTIONS`.
```

Be careful with nested markdown fences in the actual edit; use a targeted replacement that preserves the surrounding section.

### 4. Create/update todo file during implementation

Create `.pi/todos/eitype-restore-token.md`:

```md
# eitype restore token persistence

## Phase 1: Token path/storage helpers
- [ ] Add XDG state path resolver
- [ ] Add token loader
- [ ] Add strict token saver with 0700 dir / 0600 file permissions

## Phase 2: eitype integration
- [ ] Switch Linux typing to `connect_portal_with_token`
- [ ] Pass saved token into portal connection
- [ ] Save returned token before typing
- [ ] Close eitype connection on save/type errors

## Phase 3: Docs
- [ ] Document restore-token path and strict persistence behavior

## Phase 4: Verify
- [ ] cargo fmt
- [ ] cargo check
- [ ] cargo clippy
```

## Verification plan

1. Run:

```sh
cargo fmt
cargo check
cargo clippy
```

2. Optional unit-style manual path checks with temporary env vars via a small temporary Rust snippet or by adding temporary debug prints and reverting them:

- `XDG_STATE_HOME=/tmp/voxter-state` should resolve to `/tmp/voxter-state/voxter/eitype-restore-token`.
- `XDG_STATE_HOME=relative` should ignore the relative value and use `$HOME/.local/state/voxter/eitype-restore-token`.
- `HOME=` with no valid `XDG_STATE_HOME` should fail typing with a clear path-resolution error.

3. Runtime KDE/Wayland test:

```sh
cargo run
```

Expected:

- First typing attempt may show Remote Control prompt.
- With "Allow restoring on future sessions" enabled, Voxter should write the restore token under XDG state.
- Subsequent launches should pass the saved token to eitype and should not prompt when the portal accepts the token.

## Risks / follow-ups

- If the portal does not return a token, there is nothing to save. Typing can still proceed because this is not an I/O failure.
- If the saved token becomes invalid, portal behavior may be to prompt again or return a connection error. This plan does not proactively delete invalid tokens because eitype exposes only the high-level connection result.
- The eitype CLI stores its own token under cache, but Voxter intentionally uses XDG state for its app-specific persisted portal restore token.
