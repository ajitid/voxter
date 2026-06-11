# Plan: Use Enigo on macOS and eitype on Linux for transcript typing

## Goal

Fix typing on KDE/Wayland/CachyOS by replacing Linux Enigo typing with the `eitype` Rust crate. Keep macOS typing on Enigo unchanged.

User decisions locked:

- macOS: continue using `enigo`.
- Linux: use `eitype` from crates.io as a Rust dependency/library, not an external CLI and not a local `../eitype` path.
- Linux fallback policy: strict `eitype` only. Do not fallback to Enigo, dotool, ydotool, wtype, clipboard, or portal code from `adf0a50`.

## Findings / references

- Current failure:
  - `Typing error: Enigo init error: no connection could be established: (no successful connection)` on KDE/Wayland.
- Current code:
  - `src/main.rs:849-877` normalizes transcript and calls `try_type()`.
  - `src/main.rs:869-877` current `try_type()` always uses Enigo.
  - `Cargo.toml:35` macOS `enigo = "0.6"`.
  - `Cargo.toml:40` Linux currently has `enigo = { version = "0.6", default-features = false, features = ["wayland"] }`.
- Old commit `adf0a50db711cb60846f4de95f4682e7a5d840ed` used XDG RemoteDesktop portal + `NotifyKeyboardKeysym`, not Enigo, but user chose `eitype` instead.
- `cargo search eitype --limit 5` confirms crates.io package:
  - `eitype = "0.2.1"` — “A wtype-like CLI tool and library for typing text using Emulated Input (EI) protocol on Wayland”.
- `cargo info eitype` confirms:
  - latest observed version: `0.2.1`
  - repository: `https://github.com/Adam-D-Lewis/eitype`
  - docs: `https://docs.rs/eitype/0.2.1`
- docs.rs API references:
  - `EiType::connect_portal(EiTypeConfig::default()) -> Result<EiType, EiTypeError>`
  - `EiType::connect_portal_with_token(config, Option<&str>) -> Result<(EiType, Option<String>), EiTypeError>`
  - `EiType::type_text(&self, text: &str) -> Result<(), EiTypeError>`
  - `EiType::close(&mut self)`
  - `EiTypeConfig::from_env()` supports XKB env overrides.

## Desired behavior

### macOS

No functional change. `try_type()` should use Enigo exactly as today.

### Linux

`try_type()` should:

1. Connect to the XDG RemoteDesktop/libei path through `eitype`.
2. Type the normalized transcript with `EiType::type_text(text)`.
3. Close the `EiType` connection explicitly.
4. Return a clear error if eitype portal connection or typing fails.
5. Not attempt any fallback.

First run may trigger a KDE/Wayland “remote control” or RemoteDesktop permission prompt. That is expected.

## Patch spec

### 1. Update `Cargo.toml`

Use `cargo add eitype --target 'cfg(target_os = "linux")'` if implementing interactively. If editing manually, change the Linux target dependencies from:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
enigo = { version = "0.6", default-features = false, features = ["wayland"] }
rdev = { git = "https://github.com/Narsil/rdev", rev = "c14f2dc5c8100a96c5d7e3013de59d6aa0b9eae2", features = ["wayland"] }
smithay-client-toolkit = "0.20.0"
wayland-client = "0.31.1"
```

To:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
eitype = "0.2.1"
rdev = { git = "https://github.com/Narsil/rdev", rev = "c14f2dc5c8100a96c5d7e3013de59d6aa0b9eae2", features = ["wayland"] }
smithay-client-toolkit = "0.20.0"
wayland-client = "0.31.1"
```

Keep macOS Enigo dependency unchanged:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
enigo = "0.6"
```

Rationale: Linux should not compile/link Enigo anymore for typing.

### 2. Split `try_type()` by platform in `src/main.rs`

Replace the current single unguarded function:

```rust
fn try_type(text: &str) -> Result<(), String> {
    use enigo::{Enigo, Keyboard, Settings};
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("Enigo init error: {e}"))?;
    enigo
        .text(text)
        .map_err(|e| format!("Enigo text error: {e}"))?;
    Ok(())
}
```

With platform-specific implementations:

```rust
#[cfg(target_os = "macos")]
fn try_type(text: &str) -> Result<(), String> {
    use enigo::{Enigo, Keyboard, Settings};

    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("Enigo init error: {e}"))?;
    enigo
        .text(text)
        .map_err(|e| format!("Enigo text error: {e}"))?;
    Ok(())
}

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

Notes:

- `EiTypeConfig::from_env()` is preferred over `default()` so users can set XKB layout env vars if needed.
- `EiType` is `!Send`/`!Sync`, but this is fine because it is created, used, and closed inside the existing typing thread.
- If `type_text` fails, `close()` still runs before returning.
- This intentionally does not persist eitype restore tokens. `eitype` docs expose `connect_portal_with_token`, but strict/simple initial integration should avoid adding new token state unless repeated prompts become a problem. If repeated prompts happen, a follow-up plan can add token persistence under `$XDG_STATE_HOME/voxter/eitype-restore-token`.

### 3. Update docs

Update `docs/linux-wayland.md` by adding a typing section after the hotkey-helper explanation.

Suggested text:

```md
## Typing on Wayland

On Linux, Voxter types transcripts through the `eitype` Rust library, which uses libei via the XDG RemoteDesktop portal. This is intended to work on KDE/Wayland where Enigo/wtype-style virtual-keyboard approaches can fail.

Install/run requirements:

- `xdg-desktop-portal`
- `xdg-desktop-portal-kde` on KDE Plasma
- a running Wayland session with portal remote-control support

The first typing attempt may show a KDE remote-control/RemoteDesktop permission prompt. Approve it to allow Voxter to type into the active window.

Keyboard layout can be influenced with eitype/XKB environment variables such as `XKB_DEFAULT_LAYOUT`, `XKB_DEFAULT_VARIANT`, `XKB_DEFAULT_MODEL`, and `XKB_DEFAULT_OPTIONS`.
```

### 4. Update todo file during execution

Create or update `.pi/todos/eitype-linux-typing.md` during implementation with phases:

```md
# eitype Linux typing

## Phase 1: Dependencies
- [ ] Add Linux `eitype` dependency
- [ ] Remove Linux `enigo` dependency

## Phase 2: Code
- [ ] Split `try_type` by platform
- [ ] Implement Linux eitype-only typing
- [ ] Keep macOS Enigo typing unchanged

## Phase 3: Docs
- [ ] Document Linux typing portal requirements

## Phase 4: Verify
- [ ] cargo fmt
- [ ] cargo check
- [ ] cargo clippy if check passes in reasonable time
```

## Verification plan

1. Run:

```sh
cargo fmt
cargo check
```

2. If compile errors arise from eitype/ashpd dependency interactions, inspect exact errors and adjust imports/API calls to match `eitype 0.2.1`.

3. If `cargo check` passes, optionally run:

```sh
cargo clippy
```

4. Runtime manual test on KDE/Wayland:

```sh
RUST_BACKTRACE=1 cargo run
```

Then record and stop. Expected first-run behavior:

- KDE may prompt for remote-control/RemoteDesktop permission.
- After approval, the transcript should type into the currently focused text field.

## Risks / follow-ups

- `eitype::connect_portal()` may prompt more often than desired. If so, follow up with `connect_portal_with_token()` and persist the restore token.
- The project already uses a privileged hotkey helper for Linux. This plan does not change that.
- `eitype` may require system portal packages and KDE portal support. This should be documented rather than hidden behind fallback behavior.
- If users run Linux under X11, eitype may not be the right backend. Per user decision, Linux remains strict eitype-only for now.
