# Plan: KDE/Linux tray via `ksni` using the existing Voxter tray icon

## Goal

Add a Linux/KDE system tray entry that mirrors the macOS tray behavior:

- Show the same Voxter tray icon artwork currently used on macOS.
- Provide a tray menu with:
  - `Type last transcript` — types the stored last transcription.
  - separator
  - `Quit` — exits Voxter cleanly.
- Do **not** reintroduce any Linux keyboard shortcut for retyping.
- Use `ksni` / KDE StatusNotifierItem, not GTK/AppIndicator.
- Be strict: if the Linux tray cannot be registered, Voxter startup should fail.

User decision recorded: **Strict tray startup failure** if StatusNotifierItem registration fails.

## References checked

- Current macOS tray implementation: `src/ui/tray.rs`
  - Builds the menu.
  - Renders the microphone/shock-mount icon with `raqote`.
  - Disables `Type last transcript` until a transcript exists.
- UI module registration: `src/ui/mod.rs`
- Linux app entry/event loop: `src/main.rs`
  - `run_app()` Linux branch creates channel, hotkey listener, audio manager, overlay, then runs `linux_event_loop()`.
  - `handle_linux_event()` processes `AppEvent::Control`, `AppEvent::Overlay`, `AppEvent::TranscriptUpdated`.
- `ksni` docs:
  - Crate overview: https://docs.rs/ksni/latest/ksni/
  - `Tray` trait: https://docs.rs/ksni/latest/ksni/trait.Tray.html
  - Blocking API: https://docs.rs/ksni/latest/ksni/blocking/index.html
  - Blocking `TrayMethods::spawn()`: https://docs.rs/ksni/latest/ksni/blocking/trait.TrayMethods.html
  - Blocking `Handle::update()` / `shutdown()`: https://docs.rs/ksni/latest/ksni/blocking/struct.Handle.html
  - `Icon` ARGB32 format: https://docs.rs/ksni/latest/ksni/struct.Icon.html
  - `StandardItem.enabled`: https://docs.rs/ksni/latest/ksni/menu/struct.StandardItem.html

## Design

### 1. Share tray icon artwork between macOS and Linux

Current `src/ui/tray.rs` both renders the icon and converts it into `tray_icon::Icon`. Linux `ksni` needs ARGB32 bytes instead.

Create a shared module, e.g. `src/ui/tray_art.rs`, that returns platform-neutral RGBA data:

```rust
pub struct TrayIconRgba {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn build_status_icon_rgba() -> TrayIconRgba { ... }
```

Move the existing raqote drawing code from `src/ui/tray.rs::build_status_icon()` into this function. Keep the visual output identical.

Important conversion details:

- Existing macOS code currently emits `[0, 0, 0, alpha]` RGBA pixels.
- `tray-icon::Icon::from_rgba()` wants RGBA.
- `ksni::Icon` wants ARGB32 network-byte-order bytes. Convert each RGBA pixel `[r, g, b, a]` into `[a, r, g, b]`, matching the `ksni::Icon` docs.

### 2. Update macOS tray to use shared art

In `src/ui/tray.rs`:

- Remove direct `raqote` drawing imports from this file if all drawing moves to `tray_art.rs`.
- Replace `build_status_icon()` body with:

```rust
fn build_status_icon() -> Result<Icon, String> {
    let icon = crate::ui::tray_art::build_status_icon_rgba();
    Icon::from_rgba(icon.rgba, icon.width, icon.height)
        .map_err(|e| format!("Failed to create tray icon RGBA data: {e}"))
}
```

This preserves macOS behavior while ensuring Linux uses the same pixels.

### 3. Add Linux tray module

Create `src/ui/linux_tray.rs` gated by Linux from `src/ui/mod.rs`.

Suggested structure:

```rust
use crate::{AppEvent, AppSender, ControlMsg};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Category, Icon, MenuItem, Status, ToolTip, Tray};

pub struct LinuxStatusTray {
    handle: Handle<VoxterLinuxTray>,
}

impl LinuxStatusTray {
    pub fn new(sender: AppSender, has_last_transcript: bool) -> Result<Self, String> { ... }

    pub fn refresh_type_item(&self, has_last_transcript: bool) {
        let _ = self.handle.update(|tray| {
            tray.has_last_transcript = has_last_transcript;
        });
    }

    pub fn shutdown(&self) {
        let _awaiter = self.handle.shutdown();
        // If ShutdownAwaiter has a blocking wait API after checking concrete docs/source, use it.
        // Otherwise letting it drop during process exit is acceptable after sending shutdown.
    }
}

struct VoxterLinuxTray {
    sender: AppSender,
    has_last_transcript: bool,
    icon: Icon,
}
```

Implement `ksni::Tray`:

```rust
impl Tray for VoxterLinuxTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "voxter".to_string()
    }

    fn title(&self) -> String {
        "Voxter".to_string()
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn status(&self) -> Status {
        Status::Active
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![self.icon.clone()]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Voxter".to_string(),
            description: "Mistral Voxtral Speech-to-Text".to_string(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Type last transcript".to_string(),
                enabled: self.has_last_transcript,
                activate: Box::new(|tray| {
                    tray.sender.send(AppEvent::Control(ControlMsg::TypeLastTranscript));
                }),
                ..Default::default()
            }.into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".to_string(),
                enabled: true,
                activate: Box::new(|tray| {
                    tray.sender.send(AppEvent::Control(ControlMsg::Quit));
                }),
                ..Default::default()
            }.into(),
        ]
    }
}
```

Implementation note: confirm exact `ToolTip` fields while coding if the struct differs; use docs/source for the installed `ksni` version.

`new()` should:

1. Build `ksni::Icon` from `tray_art::build_status_icon_rgba()` by rotating RGBA to ARGB.
2. Create `VoxterLinuxTray`.
3. Call `tray.spawn()` from `ksni::blocking::TrayMethods`.
4. Return `Err(...)` if `spawn()` fails. Do **not** fall back.

Do **not** use `assume_sni_available(true)` because the user requested strict failure. `spawn()` should fail if no watcher is available.

### 4. Register Linux tray module

In `src/ui/mod.rs`, add:

```rust
pub mod tray_art;

#[cfg(target_os = "linux")]
pub mod linux_tray;
```

`tray_art` should be available on both macOS and Linux. If Windows is not targeted, either leave it unconditional or gate it with `#[cfg(any(target_os = "macos", target_os = "linux"))]`.

### 5. Re-enable type-last-transcript logic for Linux tray only

Recent Linux shortcut removal may have made these functions macOS-only:

```rust
#[cfg(target_os = "macos")]
fn last_transcription_text() -> Option<String> { ... }

#[cfg(target_os = "macos")]
fn type_last_transcript() -> Result<bool, String> { ... }
```

Change them to be available for both supported desktop targets:

```rust
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn last_transcription_text() -> Option<String> { ... }

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn type_last_transcript() -> Result<bool, String> { ... }
```

Or remove the cfg entirely if no unsupported target builds this binary.

Also ensure `ControlMsg::TypeLastTranscript` is available on Linux:

```rust
#[cfg(any(target_os = "macos", target_os = "linux"))]
TypeLastTranscript,
```

Do **not** add `type_last_transcript` back to the Linux hotkey helper event parser.

### 6. Add Linux event handling for tray menu action

In Linux `handle_linux_event()` in `src/main.rs`, add a `ControlMsg::TypeLastTranscript` match arm:

```rust
ControlMsg::TypeLastTranscript => match type_last_transcript() {
    Ok(true) => println!("Typed last transcript"),
    Ok(false) => println!("No last transcript available to type"),
    Err(e) => eprintln!("Failed to type transcript: {e}"),
},
```

This restores the action only through the tray, not through the global hotkey helper.

### 7. Keep Linux tray state in sync

The macOS tray disables/enables the type item via `refresh_tray_menu_state()`.

For Linux:

- Create the tray before entering the Linux event loop.
- Pass a mutable/reference handle into `linux_event_loop()` and `handle_linux_event()`.

Suggested signatures:

```rust
fn handle_linux_event(
    event: AppEvent,
    sender: &AppSender,
    audio_manager: &mut AudioManager,
    overlay: &mut LinuxOverlayController,
    tray: &LinuxStatusTray,
) -> Result<bool, Box<dyn std::error::Error>>
```

```rust
fn linux_event_loop(
    rx: std::sync::mpsc::Receiver<AppEvent>,
    sender: AppSender,
    audio_manager: &mut AudioManager,
    overlay: &mut LinuxOverlayController,
    tray: &LinuxStatusTray,
) -> Result<(), Box<dyn std::error::Error>>
```

Then in `AppEvent::TranscriptUpdated`:

```rust
tray.refresh_type_item(last_transcription_text().is_some());
```

On `ControlMsg::TypeLastTranscript`, refresh too, even though typing does not consume the transcript. This keeps behavior robust if state changes later:

```rust
tray.refresh_type_item(last_transcription_text().is_some());
```

On `ControlMsg::Quit`, before returning `Ok(false)`:

```rust
tray.shutdown();
```

### 8. Create Linux tray in `run_app()` strictly

In Linux `run_app()`:

```rust
use ui::linux_tray::LinuxStatusTray;
```

Current flow:

```rust
spawn_hotkey_listener(sender.clone());

let mut audio_manager = AudioManager::new();
let mut overlay = LinuxOverlayController::new(...)?;
linux_event_loop(rx, sender, &mut audio_manager, &mut overlay)
```

Change to:

```rust
spawn_hotkey_listener(sender.clone());

let tray = LinuxStatusTray::new(sender.clone(), last_transcription_text().is_some())
    .map_err(|e| format!("Failed to start Linux status tray: {e}"))?;

let mut audio_manager = AudioManager::new();
let mut overlay = LinuxOverlayController::new(Arc::clone(&audio_manager.speech_viz))?;
linux_event_loop(rx, sender, &mut audio_manager, &mut overlay, &tray)
```

Because startup must fail strictly, do not catch and continue after `LinuxStatusTray::new()` failure.

### 9. Cargo dependency

Prefer using cargo to add the dependency:

```sh
cargo add ksni --target 'cfg(target_os = "linux")' --features blocking
```

If cargo-add places it differently than desired, ensure final `Cargo.toml` has:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
ksni = { version = "0.3", features = ["blocking"] }
```

### 10. Documentation updates

Update `docs/linux-wayland.md`:

- Mention KDE/StatusNotifierItem tray startup is required.
- Mention the tray menu provides `Type last transcript` and `Quit`.
- Mention failure behavior: Voxter exits if the Linux tray cannot be registered.
- Keep helper docs limited to:

```json
{"event":"hotkey_press"}
{"event":"hotkey_release"}
```

Do not document any Linux retype shortcut.

### 11. Verification

Run:

```sh
cargo fmt
cargo check
cargo check --bin voxter-hotkey-helper
cargo clippy --all-targets
```

Manual KDE/Wayland test:

1. Start Voxter on KDE Plasma Wayland.
2. Confirm tray icon appears with the same artwork as macOS.
3. Confirm initial `Type last transcript` is disabled if no transcription exists.
4. Record/transcribe once.
5. Confirm `Type last transcript` becomes enabled.
6. Click it and confirm the last transcript is typed into the active window.
7. Confirm `Quit` exits, hides overlay, shuts down Linux typing worker, and unregisters tray.
8. Temporarily run without a StatusNotifier watcher/panel if possible and confirm startup fails loudly.

## Non-goals

- Do not restore or add a Linux keyboard shortcut for retyping.
- Do not add GTK/AppIndicator/`tray-icon` Linux support.
- Do not build a KDE plasmoid.
- Do not add fallback behavior when `ksni` tray registration fails; user explicitly selected strict failure.
