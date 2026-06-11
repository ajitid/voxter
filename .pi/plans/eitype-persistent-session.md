# Plan: Reuse one persistent eitype RemoteDesktop session on Linux

## Goal

Avoid repeated KDE “Remote control session started” notifications by opening the eitype/libei RemoteDesktop session lazily once, then reusing it for all Linux typing requests until Voxter exits.

This builds on `.pi/plans/eitype-restore-token.md`: restore tokens still handle cross-launch permission persistence, while this plan reduces per-transcription session churn within one Voxter process.

## User decision locked

- Implementation approach: **persistent eitype session**.
- Reuse one Linux eitype session for the app lifetime, reconnecting only after session/type errors.

## References / findings

- Current Linux typing implementation: `src/main.rs:1030-1061` creates a new `EiType` connection for every `try_type()`, saves any returned restore token, types text, then calls `typer.close()`. This start/close cycle is what can repeatedly trigger KDE portal notifications.
- Current typing call path:
  - `transcribe_audio_opus()` stores transcript and calls `type_transcript()` (`src/main.rs:760-824`).
  - `type_transcript()` spawns a detached thread and calls `try_type()` (`src/main.rs:847-866`).
  - On Linux, those detached caller threads can overlap, so the new implementation should serialize access to one eitype session.
- Current Linux quit path: `handle_linux_event(... ControlMsg::Quit ...)` hides the overlay and returns `Ok(false)` (`src/main.rs:1537-1596`). This is the best explicit place to shut down the persistent typing worker.
- `eitype 0.2.1` local source:
  - `~/.cargo/registry/src/index.crates.io-*/eitype-0.2.1/src/lib.rs:957-963` exposes `EiType::connect_portal_with_token(config, Option<&str>) -> Result<(EiType, Option<String>), EiTypeError>`.
  - `src/lib.rs:1351-1356` has `EiType::type_text(&self, text: &str)`, so typing itself does not require mutable access.
  - `src/lib.rs:1441-1466` has explicit `EiType::close(&mut self)` and `Drop` also calls `close()`, but explicit shutdown is cleaner and should reduce lingering portal/session state during graceful app quit.
- XDG RemoteDesktop portal docs:
  - RemoteDesktop creates/starts sessions; `ConnectToEIS` is called after `Start` and only once per session.
  - `SelectDevices` supports `persist_mode` and `restore_token`; `Start` may return a replacement `restore_token`.
  - Source: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html
- RustDesk community discussion shows remote-control apps commonly keep the OS remote-control session alive while the app is running, but notes the always-active indicator can be misleading.
  - Source: https://github.com/rustdesk/rustdesk/discussions/13902

## Design

Use a Linux-only typing worker thread that owns the `EiType` instance.

Why worker thread instead of `static Mutex<Option<EiType>>`:

- Avoids depending on whether all internals of `EiType` are `Send + Sync` in a static global.
- Serializes typing requests from multiple detached `type_transcript()` caller threads.
- Keeps session lifecycle in one place: connect lazily, reuse, close on shutdown/error.

Behavior:

1. First Linux `try_type(text)` sends a request to the worker.
2. Worker lazily connects via `connect_portal_with_token()` using the saved restore token.
3. Worker saves any returned token using existing strict token I/O helpers.
4. Worker calls `type_text(text)` on the cached `EiType`.
5. On success, keep the session open for future requests.
6. On typing error, call `close()`, drop cached session, and return the error. Do **not** retry the same text automatically, to avoid duplicating partial text.
7. Next request reconnects lazily.
8. On app quit, send `Shutdown`; worker closes the cached session and exits.

Strict token policy remains unchanged:

- Path/read/write/chmod/rename failures should fail typing clearly.
- Missing token file remains allowed.
- If saving a newly returned token fails during connection, close the newly opened portal session and fail that typing request.

## Patch spec

### 1. Add Linux typing-worker message types and worker state in `src/main.rs`

Immediately before the existing Linux `try_type()` function, after `save_eitype_restore_token()`, add:

```rust
#[cfg(target_os = "linux")]
enum LinuxTypingMsg {
    Type {
        text: String,
        response: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    Shutdown,
}

#[cfg(target_os = "linux")]
static LINUX_TYPING_WORKER: std::sync::OnceLock<Mutex<Option<std::sync::mpsc::Sender<LinuxTypingMsg>>>> =
    std::sync::OnceLock::new();
```

Notes:

- `Mutex` is already imported at the top as `use std::sync::Mutex;`.
- Use fully-qualified `std::sync::mpsc` types to avoid top-level import churn.
- The static stores the sender, not `EiType`; the worker thread owns `EiType`.

### 2. Add Linux worker helpers in `src/main.rs`

Add these helpers after the static from step 1 and before Linux `try_type()`:

```rust
#[cfg(target_os = "linux")]
fn connect_eitype_with_saved_token() -> Result<eitype::EiType, String> {
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

    if let Some(token) = new_token.as_deref()
        && let Err(error) = save_eitype_restore_token(token)
    {
        typer.close();
        return Err(error);
    }

    Ok(typer)
}

#[cfg(target_os = "linux")]
fn run_linux_typing_worker(rx: std::sync::mpsc::Receiver<LinuxTypingMsg>) {
    let mut typer: Option<eitype::EiType> = None;

    while let Ok(message) = rx.recv() {
        match message {
            LinuxTypingMsg::Type { text, response } => {
                let result = (|| {
                    if typer.is_none() {
                        typer = Some(connect_eitype_with_saved_token()?);
                    }

                    let active_typer = typer
                        .as_ref()
                        .ok_or_else(|| "eitype worker has no active connection".to_string())?;

                    if let Err(error) = active_typer
                        .type_text(&text)
                        .map_err(|e| format!("eitype text error: {e}"))
                    {
                        if let Some(mut stale_typer) = typer.take() {
                            stale_typer.close();
                        }
                        return Err(error);
                    }

                    Ok(())
                })();

                let _ = response.send(result);
            }
            LinuxTypingMsg::Shutdown => break,
        }
    }

    if let Some(mut active_typer) = typer {
        active_typer.close();
    }
}

#[cfg(target_os = "linux")]
fn linux_typing_worker_sender() -> std::sync::mpsc::Sender<LinuxTypingMsg> {
    let worker_slot = LINUX_TYPING_WORKER.get_or_init(|| Mutex::new(None));
    let mut worker = worker_slot
        .lock()
        .expect("Linux eitype typing worker mutex poisoned");

    if let Some(sender) = worker.as_ref() {
        return sender.clone();
    }

    let (tx, rx) = std::sync::mpsc::channel::<LinuxTypingMsg>();
    std::thread::spawn(move || run_linux_typing_worker(rx));
    *worker = Some(tx.clone());
    tx
}

#[cfg(target_os = "linux")]
fn shutdown_linux_typing_worker() {
    let Some(worker_slot) = LINUX_TYPING_WORKER.get() else {
        return;
    };

    let mut worker = match worker_slot.lock() {
        Ok(worker) => worker,
        Err(error) => {
            eprintln!("Linux eitype typing worker mutex poisoned during shutdown: {error}");
            return;
        }
    };

    if let Some(sender) = worker.take() {
        let _ = sender.send(LinuxTypingMsg::Shutdown);
    }
}
```

Important implementation notes:

- `run_linux_typing_worker()` should own and reuse `Option<eitype::EiType>`.
- On `type_text()` failure, close and drop the cached session, but do not retry the same text.
- On token-save failure in `connect_eitype_with_saved_token()`, close the just-opened session and return the save error.
- `shutdown_linux_typing_worker()` intentionally does not block/join; sending shutdown is enough for graceful normal quit, and process exit remains the ultimate cleanup fallback.

### 3. Replace Linux `try_type()` in `src/main.rs`

Replace the current Linux `try_type()` body with a request/response send to the worker:

```rust
#[cfg(target_os = "linux")]
fn try_type(text: &str) -> Result<(), String> {
    let (response_tx, response_rx) = std::sync::mpsc::sync_channel(1);
    let sender = linux_typing_worker_sender();

    sender
        .send(LinuxTypingMsg::Type {
            text: text.to_string(),
            response: response_tx,
        })
        .map_err(|_| "Linux eitype typing worker is not running".to_string())?;

    response_rx
        .recv()
        .map_err(|_| "Linux eitype typing worker exited before returning a result".to_string())?
}
```

This preserves the existing external behavior: `type_transcript()` still runs typing in a detached thread and logs any `try_type()` error.

### 4. Ensure Linux quit closes the persistent session

In `handle_linux_event()` Linux `ControlMsg::Quit` arm, add `shutdown_linux_typing_worker();` before hiding the overlay / returning false.

Current block:

```rust
            ControlMsg::Quit => {
                if audio_manager.recorder.is_recording()
                    && let Err(e) = audio_manager.stop_recording(sender.clone())
                {
                    eprintln!("Failed to stop recording: {e}");
                }
                overlay.update_state(OverlayState::Hidden)?;
                return Ok(false);
            }
```

Replace with:

```rust
            ControlMsg::Quit => {
                if audio_manager.recorder.is_recording()
                    && let Err(e) = audio_manager.stop_recording(sender.clone())
                {
                    eprintln!("Failed to stop recording: {e}");
                }
                shutdown_linux_typing_worker();
                overlay.update_state(OverlayState::Hidden)?;
                return Ok(false);
            }
```

### 5. Update `docs/linux-wayland.md`

In `## Typing on Wayland`, after the paragraph documenting strict token persistence, add:

```md
Within a single Voxter run, Voxter keeps one eitype RemoteDesktop session open and reuses it for subsequent typing requests. This avoids starting a new portal session for every transcription, which reduces repeated KDE "Remote control session started" notifications. KDE may still show an active remote-control indicator while Voxter is running; quit Voxter to close the session.
```

### 6. Create/update implementation todo file

Create `.pi/todos/eitype-persistent-session.md` during implementation:

```md
# eitype persistent session

## Phase 1: Worker lifecycle
- [ ] Add Linux typing worker message enum/static
- [ ] Add lazy worker startup helper
- [ ] Add shutdown helper

## Phase 2: Persistent eitype connection
- [ ] Move connect/token-save logic into reusable connect helper
- [ ] Cache one `EiType` inside the worker
- [ ] Reuse cached session for successive typing requests
- [ ] Close/drop cached session on typing errors without retrying same text

## Phase 3: App integration
- [ ] Replace Linux `try_type()` with worker request/response
- [ ] Shut down worker on Linux quit

## Phase 4: Docs
- [ ] Document persistent session behavior and KDE indicator tradeoff

## Phase 5: Verify
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

2. Manual runtime test on KDE/Wayland:

```sh
cargo run
```

Expected:

- First typing after launch may start one KDE RemoteDesktop session and may show one notification/indicator.
- Later transcriptions during the same Voxter run should reuse the session and should not show a new “session started” notification each time.
- Quitting Voxter should close the session; KDE’s active remote-control indicator should disappear shortly after quit.

3. Reconnect behavior test:

- Start Voxter and type once.
- Manually revoke/interrupt portal session if possible, or kill/restart portal/KWin in a safe test session.
- Next typing error should be logged, cached eitype should be dropped, and a later typing attempt should reconnect lazily.

## Risks / follow-ups

- KDE may still show a persistent active remote-control indicator while Voxter runs. This is expected because the session is intentionally kept open.
- If the keyboard layout changes after the persistent eitype connection is established, eitype may keep using the keymap/layout detected at connection time. If this becomes a problem, add an explicit reconnect action or reconnect-on-layout-change strategy later.
- If `type_text()` partially emits text before returning an error, automatic retry could duplicate partial output; therefore this plan intentionally reconnects only for future requests.
- `shutdown_linux_typing_worker()` is best-effort and non-blocking. If strict join-on-exit is desired later, store a `JoinHandle` in the worker static too, but that adds more lifecycle complexity for little benefit.
