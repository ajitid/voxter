# Plan: Continue on GNOME when `zwlr_layer_shell_v1` overlay is unavailable

## Goal

On Linux/Wayland, Voxter should try to initialize the current `smithay-client-toolkit` / `zwlr_layer_shell_v1` overlay, but if that fails (as on GNOME/Mutter), the app should continue running without overlay UI.

User choices locked:

- Detection behavior: **try overlay everywhere; if layer-shell/overlay init fails, continue without overlay**.
- Replacement feedback: **none for now; keep sounds and tray only**.

## Context / references

Repo references:

- Linux overlay implementation: `src/ui/linux_overlay.rs`
  - `LinuxOverlayController::new(...)` fails if Wayland/layer-shell requirements are unavailable.
  - It binds `LayerShell::bind(&globals, &qh)`, which requires `zwlr_layer_shell_v1`.
- Linux app startup: `src/main.rs`
  - `run_app()` currently exits if overlay init fails:
    ```rust
    let mut overlay = LinuxOverlayController::new(Arc::clone(&audio_manager.speech_viz))?;
    linux_event_loop(rx, sender, &mut audio_manager, &mut overlay, &tray)
    ```
  - `linux_event_loop(...)` and `handle_linux_event(...)` currently require `&mut LinuxOverlayController`.
- Linux tray remains strict: `src/ui/linux_tray.rs` via `LinuxStatusTray::new(...)`.

Web findings from prior research:

- GNOME/Mutter generally does not implement `zwlr_layer_shell_v1`; wlroots/layer-shell apps fail there.
- `gtk-layer-shell`/`gtk4-layer-shell` would not help because they still require the same layer-shell protocol.
- AppIndicator/KStatusNotifierItem tray support can work on GNOME via the Ubuntu AppIndicator extension, but that is separate from the overlay.

## Behavior after change

- If `WAYLAND_DISPLAY` is unset: keep current hard error. The Linux build is still Wayland-only for now.
- If installed assets are missing: keep current hard error.
- If tray registration fails: keep current hard error.
- If overlay initialization fails:
  - print a clear warning to stderr;
  - continue running with `overlay: None`;
  - ignore future `AppEvent::Overlay(...)` render/update work;
  - still reset speech visualization state when hidden, because that state belongs to recording logic too;
  - use slower event-loop timeout because there is no overlay animation to redraw.

## Implementation patch spec

### 1. Change Linux event handler to accept optional overlay

File: `src/main.rs`

Find the Linux `handle_linux_event` signature:

```rust
fn handle_linux_event(
    event: AppEvent,
    sender: &AppSender,
    audio_manager: &mut AudioManager,
    overlay: &mut LinuxOverlayController,
    tray: &LinuxStatusTray,
) -> Result<bool, Box<dyn std::error::Error>> {
```

Replace `overlay: &mut LinuxOverlayController,` with:

```rust
overlay: Option<&mut LinuxOverlayController>,
```

Inside the function, make `overlay` mutable at the start so it can be reborrowed multiple times:

```rust
let mut overlay = overlay;
```

Exact location: first line after the opening `{` of `handle_linux_event`.

### 2. Guard overlay calls in `handle_linux_event`

In `ControlMsg::Quit`, replace:

```rust
overlay.update_state(OverlayState::Hidden)?;
```

with:

```rust
if let Some(overlay) = overlay.as_deref_mut() {
    overlay.update_state(OverlayState::Hidden)?;
}
```

In `AppEvent::Overlay(state)`, replace:

```rust
if state == OverlayState::Hidden {
    audio_manager.speech_viz.reset();
}
overlay.update_state(state)?;
```

with:

```rust
if state == OverlayState::Hidden {
    audio_manager.speech_viz.reset();
}
if let Some(overlay) = overlay.as_deref_mut() {
    overlay.update_state(state)?;
}
```

This makes overlay events no-ops when overlay is disabled, without changing recording/transcription behavior.

### 3. Change Linux event loop to store optional overlay

File: `src/main.rs`

Find `linux_event_loop(...)` signature:

```rust
fn linux_event_loop(
    rx: std::sync::mpsc::Receiver<AppEvent>,
    sender: AppSender,
    audio_manager: &mut AudioManager,
    overlay: &mut LinuxOverlayController,
    tray: &LinuxStatusTray,
) -> Result<(), Box<dyn std::error::Error>> {
```

Replace `overlay: &mut LinuxOverlayController,` with:

```rust
overlay: Option<&mut LinuxOverlayController>,
```

At the start of the function body, add:

```rust
let mut overlay = overlay;
```

Then replace unconditional dispatch:

```rust
overlay.dispatch_pending()?;
```

with:

```rust
if let Some(overlay) = overlay.as_deref_mut() {
    overlay.dispatch_pending()?;
}
```

Replace timeout calculation:

```rust
let timeout = if overlay.is_visible() {
    std::time::Duration::from_millis(16)
} else {
    std::time::Duration::from_millis(250)
};
```

with:

```rust
let timeout = if overlay.as_ref().is_some_and(|overlay| overlay.is_visible()) {
    std::time::Duration::from_millis(16)
} else {
    std::time::Duration::from_millis(250)
};
```

Replace each call to `handle_linux_event(..., overlay, tray)` with a reborrowed optional overlay:

```rust
handle_linux_event(event, &sender, audio_manager, overlay.as_deref_mut(), tray)?
```

There are two call sites inside `linux_event_loop`.

Replace final redraw:

```rust
if overlay.is_visible() {
    overlay.redraw_if_visible()?;
}
```

with:

```rust
if let Some(overlay) = overlay.as_deref_mut()
    && overlay.is_visible()
{
    overlay.redraw_if_visible()?;
}
```

If clippy complains about the chained `if let` style on the project’s Rust version, use nested `if` instead.

### 4. Make overlay initialization non-fatal in Linux startup

File: `src/main.rs`

In Linux `run_app()`, replace:

```rust
let mut audio_manager = AudioManager::new();
let mut overlay = LinuxOverlayController::new(Arc::clone(&audio_manager.speech_viz))?;
linux_event_loop(rx, sender, &mut audio_manager, &mut overlay, &tray)
```

with:

```rust
let mut audio_manager = AudioManager::new();
let mut overlay = match LinuxOverlayController::new(Arc::clone(&audio_manager.speech_viz)) {
    Ok(overlay) => Some(overlay),
    Err(error) => {
        eprintln!(
            "Linux overlay is unavailable; continuing without overlay UI. Reason: {error}"
        );
        None
    }
};
linux_event_loop(rx, sender, &mut audio_manager, overlay.as_mut(), &tray)
```

### 5. Update Linux docs

File: `docs/linux-wayland.md`

Add a new subsection, likely after the strict startup paragraph or before “Typing on Wayland”:

```md
## Overlay availability

Voxter tries to show its recording overlay through the Wayland `zwlr_layer_shell_v1` protocol. Some compositors, including GNOME/Mutter, do not provide this protocol. If overlay initialization fails, Voxter continues without overlay UI; recording sounds and the tray menu still work.
```

Also update the top strict-startup sentence from:

```md
Startup is strict: if Voxter cannot find installed Linux assets or cannot register the StatusNotifierItem tray, it exits instead of continuing without a tray/icon/sounds.
```

Keep this sentence as-is or explicitly note overlay is not strict:

```md
Startup is strict for installed assets and the StatusNotifierItem tray. The recording overlay is best-effort: if the compositor does not provide the needed layer-shell protocol, Voxter continues without overlay UI.
```

### 6. Verification

Run:

```sh
cargo fmt
cargo check --all-targets
cargo clippy --all-targets
```

Optional local runtime verification on GNOME:

```sh
WAYLAND_DISPLAY=$WAYLAND_DISPLAY cargo run --bin voxter
```

Expected on GNOME/Mutter:

- app no longer exits due to `zwlr_layer_shell_v1 is not available`;
- stderr logs the overlay warning;
- tray still registers if AppIndicator/KStatusNotifierItem support is installed;
- hotkey, sounds, transcription, and typing paths are unchanged.

## Non-goals

- No tray icon color/status changes in this plan.
- No libnotify replacement feedback in this plan.
- No removal of `smithay-client-toolkit` / `wayland-client` dependencies; they remain used on KDE/wlroots desktops where layer-shell works.
- No GNOME Shell extension work.
