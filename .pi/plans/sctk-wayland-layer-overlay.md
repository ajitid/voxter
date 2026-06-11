# Plan: Replace Linux overlay with Smithay Client Toolkit layer-shell

## Goal

Fix KDE/Wayland Linux overlay behavior by replacing the Linux `winit` overlay window with a `smithay-client-toolkit` `wlr-layer-shell` overlay rendered through Wayland SHM buffers using `raqote` CPU drawing.

This intentionally makes the Linux overlay **Wayland-only** and **fail-fast** if `WAYLAND_DISPLAY` or `zwlr_layer_shell_v1` is unavailable. No X11/winit fallback for Linux.

## User decisions locked

- Rendering path: **SCTK + SHM buffers + raqote CPU drawing**.
- Linux fallback policy: **Wayland-only Linux overlay; fail fast if layer-shell is unavailable**.

## Why this fixes the bug

Current Linux overlay uses `winit::Window` in `src/ui/overlay.rs`:

```rust
.with_visible(false)
...
self.window.set_visible(false);
self.window.set_visible(true);
```

In `winit 0.30.13`, Wayland `set_visible` is a no-op:

- Local source: `~/.cargo/registry/src/.../winit-0.30.13/src/platform_impl/linux/wayland/window/mod.rs`
- Relevant implementation:

```rust
pub fn set_visible(&self, _visible: bool) {
    // Not possible on Wayland.
}
```

Also, normal `winit` Wayland windows are `xdg_toplevel`, so KDE/KWin includes them in Alt-Tab.

Layer-shell surfaces are not normal toplevel windows; they are appropriate for overlays/panels/OSDs and should not be Alt-Tabbable.

## References checked

- SCTK layer shell docs: `smithay_client_toolkit::shell::wlr_layer`
  - `LayerShell`, `LayerSurface`, `Layer`, `Anchor`, `KeyboardInteractivity`, `LayerShellHandler`
  - URL: `https://smithay.github.io/client-toolkit/smithay_client_toolkit/shell/wlr_layer/index.html`
- SCTK example:
  - URL: `https://raw.githubusercontent.com/Smithay/client-toolkit/master/examples/simple_layer.rs`
  - Shows `Connection::connect_to_env`, `registry_queue_init`, `LayerShell::bind`, `Shm::bind`, `SlotPool`, frame callbacks, and `buffer.attach_to(...); layer.commit()`.
- Wayland layer-shell protocol docs:
  - URL: `https://wayland.app/protocols/wlr-layer-shell-unstable-v1`
- KDE Plasma shell protocol notes:
  - URL: `https://wayland.app/protocols/kde-plasma-shell`
  - KDE-specific overlay hints are deprecated in Plasma 6; apps should use layer-shell where appropriate.
- Package availability on CachyOS/Arch verified locally:
  - `gtk4-layer-shell` installed, but not used for this plan.
  - `layer-shell-qt` installed, but not used for this plan.
  - SCTK is a Rust crate, no extra system library beyond Wayland client protocols expected.

## Dependencies

Update `Cargo.toml`:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
enigo = { version = "0.6", default-features = false, features = ["wayland"] }
rdev = { git = "https://github.com/Narsil/rdev", rev = "c14f2dc5c8100a96c5d7e3013de59d6aa0b9eae2", features = ["wayland"] }
smithay-client-toolkit = "0.20.0"
wayland-client = "0.31.1"
```

Notes:

- SCTK re-exports some crates, but the example imports `wayland_client` directly. Add `wayland-client` explicitly for clarity.
- SCTK default features include `calloop`, but this plan can start with plain `EventQueue::blocking_dispatch`/`dispatch_pending` plus std channels. If implementation prefers calloop later, it can use SCTK reexports.

## Architecture changes

### Current architecture

Linux and macOS both run a `winit` `ApplicationHandler<AppEvent>` in `src/main.rs`.

- `App` owns `AudioManager` and `OverlayController`.
- Hotkey thread sends `AppEvent::Control(...)` through `EventLoopProxy`.
- Transcription threads send overlay updates through the same `EventLoopProxy`.
- `OverlayController` owns a `winit::Window` and `OverlayRenderer`.

### Target architecture

Use platform-specific app loops:

- macOS: keep current `winit` app path and `src/ui/overlay.rs` + `src/ui/render.rs`.
- Linux: use a custom main-thread loop with:
  - `std::sync::mpsc::Sender<AppEvent>` / `Receiver<AppEvent>` instead of `winit::EventLoopProxy`.
  - `AudioManager` still on the main thread.
  - New SCTK overlay controller owned on the main thread.
  - Hotkey helper thread sends `AppEvent::Control(...)` through `mpsc::Sender`.
  - Transcription threads send overlay updates through a small cross-platform sender abstraction.

Important: keep `AudioManager` on the main thread because `cpal::Stream` is not Send/Sync per project notes.

## Implementation phases

### Phase 1: Introduce platform event sender abstraction

Edit `src/main.rs` around the current `AppEvent` definition.

Add a cloneable sender type:

```rust
#[derive(Clone)]
enum AppSender {
    #[cfg(target_os = "macos")]
    Winit(EventLoopProxy<AppEvent>),
    #[cfg(target_os = "linux")]
    Channel(std::sync::mpsc::Sender<AppEvent>),
}

impl AppSender {
    fn send(&self, event: AppEvent) {
        match self {
            #[cfg(target_os = "macos")]
            AppSender::Winit(proxy) => {
                let _ = proxy.send_event(event);
            }
            #[cfg(target_os = "linux")]
            AppSender::Channel(tx) => {
                let _ = tx.send(event);
            }
        }
    }
}
```

Then change these signatures/usages:

- `AudioManager::stop_recording(&mut self, proxy: EventLoopProxy<AppEvent>)` -> `AudioManager::stop_recording(&mut self, sender: AppSender)`
- Internal clones become `let sender_clone = sender.clone();`
- Replace `proxy.send_event(...)` with `sender.send(...)`.
- `spawn_hotkey_listener(proxy: EventLoopProxy<AppEvent>)` -> `spawn_hotkey_listener(sender: AppSender)` on both platforms.

For macOS `App`:

- Change field `proxy: EventLoopProxy<AppEvent>` to `sender: AppSender`.
- `App::new(proxy)` should wrap `AppSender::Winit(proxy)`.
- Replace `self.proxy.send_event(...)` with `self.sender.send(...)`.

Exact high-impact call sites currently visible:

- `src/main.rs:497` `fn stop_recording(&mut self, proxy: EventLoopProxy<AppEvent>)`
- `src/main.rs:532`, `537`, `573`, `582`, `586`, `592`, `598`, `603` overlay sends inside stop/transcription flow
- `src/main.rs:1043`, `1052`, `1062`, `1069`, `1076`, `1082` control handling sends
- `src/main.rs:1178` macOS `spawn_hotkey_listener`
- `src/main.rs:1211` Linux `spawn_hotkey_listener`
- `src/main.rs:1335` Ctrl+C handler
- `src/main.rs:1344` tray menu handler

### Phase 2: Extract reusable raqote overlay painter

Create `src/ui/painter.rs`.

Move/copy the CPU drawing part of `src/ui/render.rs` into a reusable painter:

```rust
pub struct OverlayPainter {
    dt: raqote::DrawTarget,
    physical_size: winit::dpi::PhysicalSize<u32>,
    logical_size: winit::dpi::LogicalSize<f32>,
    scale_factor: f64,
    speech_viz: Arc<SpeechVizState>,
    last_state: OverlayState,
    state_started_at: Instant,
}
```

Methods:

- `new(physical_size, scale_factor, speech_viz) -> Self`
- `resize(physical_size, scale_factor)`
- `draw_frame(state, now)`
- `data_u8(&self) -> &[u8]`
- `physical_size(&self) -> PhysicalSize<u32>`

Move these helper methods from `OverlayRenderer` into `OverlayPainter`:

- `draw_polyline_source`
- `soft_premium_gradient`
- `arc_gradient_source`
- `draw_latch_lock_icon`
- `arc_points_from_sagitta`
- `draw_sine_squares`

Then update `src/ui/render.rs`:

- Add `use crate::ui::painter::OverlayPainter;`
- Replace renderer fields:
  - remove `dt`, `physical_size`, `logical_size`, `scale_factor`, `speech_viz`, `last_state`, `state_started_at`
  - add `painter: OverlayPainter`
- `OverlayRenderer::new` creates `OverlayPainter::new(physical_size, scale_factor, speech_viz)`.
- `resize` calls `self.painter.resize(...)` and uses painter size for texture recreation.
- `draw_frame` becomes:

```rust
self.painter.draw_frame(state, now);
let bytes = self.painter.data_u8();
self.queue.write_texture(... bytes ...);
```

Keep macOS behavior unchanged after refactor.

Update `src/ui/mod.rs`:

```rust
pub mod painter;
```

### Phase 3: Add Linux SCTK overlay module

Create `src/ui/linux_overlay.rs` behind cfg Linux:

```rust
#[cfg(target_os = "linux")]
pub mod linux_overlay;
```

Core public API:

```rust
pub struct LinuxOverlayController { ... }

impl LinuxOverlayController {
    pub fn new(speech_viz: Arc<SpeechVizState>) -> Result<Self, String>;
    pub fn update_state(&mut self, next_state: OverlayState) -> Result<(), String>;
    pub fn is_visible(&self) -> bool;
    pub fn dispatch_pending(&mut self) -> Result<(), String>;
    pub fn redraw_if_visible(&mut self) -> Result<(), String>;
}
```

Internal SCTK state should follow `simple_layer.rs`:

Imports:

```rust
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, EventQueue, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};
```

Suggested object split:

```rust
struct LinuxOverlayController {
    conn: Connection,
    event_queue: EventQueue<LayerApp>,
    qh: QueueHandle<LayerApp>,
    app: LayerApp,
}

struct LayerApp {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    compositor: CompositorState,
    layer_shell: LayerShell,
    pool: SlotPool,
    layer: Option<LayerSurface>,
    painter: OverlayPainter,
    state: OverlayState,
    visible_since: Option<Instant>,
    first_configure: bool,
    configured_width: u32,
    configured_height: u32,
    needs_redraw: bool,
}
```

Layer creation on show:

```rust
let surface = self.compositor.create_surface(qh);
let layer = self.layer_shell.create_layer_surface(
    qh,
    surface,
    Layer::Overlay,
    Some("voxter-overlay"),
    None,
);
layer.set_anchor(Anchor::BOTTOM);
layer.set_keyboard_interactivity(KeyboardInteractivity::None);
layer.set_size(420, 96);
layer.set_margin(0, 0, 0, 0); // verify exact SCTK 0.20 method signature during implementation
layer.commit(); // initial empty commit
self.layer = Some(layer);
self.first_configure = true;
```

If `Layer::Overlay` appears above fullscreen in an undesirable way on KWin, change to `Layer::Top`. Start with `Overlay` because this is an OSD-like transient overlay.

Hide behavior:

- Destroy/drop `LayerSurface` by taking `self.layer = None`.
- Set state to `Hidden`.
- Set `visible_since = None`.
- Do not keep a transparent always-mapped surface.

Configure handling:

- In `LayerShellHandler::configure`, set `configured_width/height` from `configure.new_size` with fallback `420x96`.
- Resize painter and SHM pool if needed.
- Mark `needs_redraw = true`.

Frame handling:

- In `CompositorHandler::frame`, if visible, mark/draw next frame.
- `redraw_if_visible` should draw at roughly 60 FPS while visible. A simple first implementation can call `draw()` each Linux main-loop tick; better is to request frame callbacks after each draw and only draw on frame callback.

SHM drawing:

```rust
let stride = width as i32 * 4;
let (buffer, canvas) = self.pool.create_buffer(
    width as i32,
    height as i32,
    stride,
    wl_shm::Format::Argb8888,
)?;
self.painter.draw_frame(self.state, Instant::now());
canvas.copy_from_slice(self.painter.data_u8());
layer.wl_surface().damage_buffer(0, 0, width as i32, height as i32);
layer.wl_surface().frame(qh, FrameCallbackData(layer.wl_surface().clone()));
buffer.attach_to(layer.wl_surface())?;
layer.commit();
```

Verify channel byte order during implementation. `raqote::DrawTarget::get_data_u8()` worked as BGRA for wgpu `Bgra8UnormSrgb`; Wayland `Argb8888` expects native-endian ARGB values in memory, commonly BGRA byte order on little-endian. If colors are swapped, convert pixels before copying or use `wl_shm::Format::Xrgb8888` only if alpha can be dropped. Since transparency is required, keep `Argb8888` and fix conversion if necessary.

Implement required SCTK traits for `LayerApp`:

- `CompositorHandler`
- `OutputHandler`
- `LayerShellHandler`
- `ShmHandler`
- `ProvidesRegistryState`

Use delegates:

```rust
delegate_registry!(LayerApp);
delegate_compositor!(LayerApp);
delegate_output!(LayerApp);
delegate_shm!(LayerApp);
delegate_layer!(LayerApp);
registry_handlers![OutputState];
```

If SCTK 0.20 requires `delegate_dispatch2!(LayerApp)` as in the latest example, add it.

### Phase 4: Add Linux main loop

In `src/main.rs`, split `main` into platform-specific runners.

Keep common startup print code if desired, but platform-specific event loops should diverge.

Mac path:

```rust
#[cfg(target_os = "macos")]
fn run_app() -> Result<(), Box<dyn std::error::Error>> { ... existing winit setup ... }
```

Linux path:

```rust
#[cfg(target_os = "linux")]
fn run_app() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err("WAYLAND_DISPLAY is not set; Voxter Linux overlay requires Wayland".into());
    }

    let (tx, rx) = std::sync::mpsc::channel::<AppEvent>();
    let sender = AppSender::Channel(tx.clone());

    ctrlc::set_handler({
        let sender = sender.clone();
        move || sender.send(AppEvent::Control(ControlMsg::Quit))
    })?;

    spawn_hotkey_listener(sender.clone());

    let mut audio_manager = AudioManager::new();
    let mut overlay = LinuxOverlayController::new(Arc::clone(&audio_manager.speech_viz))?;

    linux_event_loop(rx, sender, &mut audio_manager, &mut overlay)
}
```

Linux event loop sketch:

```rust
fn linux_event_loop(
    rx: std::sync::mpsc::Receiver<AppEvent>,
    sender: AppSender,
    audio_manager: &mut AudioManager,
    overlay: &mut LinuxOverlayController,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut running = true;
    while running {
        overlay.dispatch_pending()?;

        let timeout = if overlay.is_visible() {
            std::time::Duration::from_millis(16)
        } else {
            std::time::Duration::from_millis(250)
        };

        match rx.recv_timeout(timeout) {
            Ok(event) => {
                running = handle_linux_event(event, &sender, audio_manager, overlay)?;
                while let Ok(event) = rx.try_recv() {
                    running = handle_linux_event(event, &sender, audio_manager, overlay)?;
                    if !running { break; }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if overlay.is_visible() {
            overlay.redraw_if_visible()?;
        }
    }
    Ok(())
}
```

`handle_linux_event` mirrors the current `App::handle_control` and `AppEvent::Overlay` branches:

- `ControlMsg::StopHold`: stop if recording and mode is Hold.
- `ControlMsg::SinglePress`: stop latch, or start hold, then send/show Recording.
- `ControlMsg::SwitchToLatch`: call `switch_to_latch_mode`, show RecordingLatch.
- `ControlMsg::Quit`: stop if needed, hide overlay, return `false`.
- `AppEvent::Overlay(Hidden)`: reset `speech_viz`, call `overlay.update_state(Hidden)`.
- `AppEvent::Overlay(state)`: call `overlay.update_state(state)`.
- `AppEvent::TranscriptUpdated`: no-op on Linux.

This removes Linux dependency on `winit::ApplicationHandler` while preserving main-thread `AudioManager` ownership.

### Phase 5: cfg-gate winit overlay/render modules to macOS where possible

Current `src/ui/mod.rs` always exposes:

```rust
pub mod overlay;
pub mod render;
```

Change to:

```rust
pub mod painter;

#[cfg(target_os = "macos")]
pub mod overlay;
#[cfg(target_os = "macos")]
pub mod render;
#[cfg(target_os = "macos")]
pub mod tray;

#[cfg(target_os = "linux")]
pub mod linux_overlay;
```

Then in `src/main.rs`, cfg imports:

```rust
#[cfg(target_os = "macos")]
use ui::overlay::OverlayController;
use ui::painter_or_overlay_state_path::OverlayState; // see next note
#[cfg(target_os = "linux")]
use ui::linux_overlay::LinuxOverlayController;
```

Move `OverlayState` out of `src/ui/overlay.rs` into `src/ui/painter.rs` or a new `src/ui/state.rs`, because Linux needs it without compiling macOS winit overlay.

Recommended:

- Create `src/ui/state.rs` containing only:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayState {
    Hidden,
    Recording,
    RecordingLatch,
    Transcribing,
}
```

- `src/ui/mod.rs`: `pub mod state;`
- Update imports:
  - `src/ui/overlay.rs`: `use crate::ui::state::OverlayState;`
  - `src/ui/render.rs`: `use crate::ui::state::OverlayState;`
  - `src/ui/painter.rs`: `use crate::ui::state::OverlayState;`
  - `src/main.rs`: `use ui::state::OverlayState;`

### Phase 6: Verification

Run:

```bash
cargo fmt
cargo check
cargo clippy
```

On KDE Wayland/CachyOS runtime checks:

```bash
WAYLAND_DEBUG=1 cargo run 2> /tmp/voxter-wayland.log
```

Expected protocol evidence:

- binds `zwlr_layer_shell_v1`
- creates `zwlr_layer_surface_v1`
- no `xdg_toplevel` for overlay

Functional checks:

1. Start app. No overlay visible initially.
2. Press/hold Super+C. Overlay appears at bottom.
3. Release. Overlay disappears during VAD; transcribing spinner appears only if transcription starts; then disappears.
4. While overlay is visible, Alt-Tab should not show `voxtral overlay`/`voxter` overlay surface as a window.
5. Press Space during hold. Lock icon appears.
6. Ctrl+C quits cleanly.

If overlay appears but is incorrectly positioned:

- Adjust `Anchor` and margins.
- Layer-shell cannot do arbitrary center positioning directly; bottom-center is achieved by setting fixed width and anchoring bottom without left/right anchors. If KWin stretches or places unexpectedly, use left+right anchors with exclusive zone 0 and draw centered inside full-width surface. That fallback is still layer-shell and non-Alt-Tabbable.

## Risks / notes

- SCTK API details may differ slightly between 0.19 docs and 0.20 crate. Use local crate source as source of truth during implementation.
- SHM buffer byte order may need a conversion from raqote bytes to Wayland `Argb8888`.
- Destroy/recreate on every show/hide is simple and correct for transient overlay. If flicker appears, optimize later by reusing surface and unmapping via null attach if SCTK exposes the needed lower-level calls.
- This plan intentionally breaks Linux X11 overlay support per user preference.

## Files to edit/create

- Edit `Cargo.toml`
- Edit `src/main.rs`
- Edit `src/ui/mod.rs`
- Edit `src/ui/render.rs`
- Edit `src/ui/overlay.rs`
- Create `src/ui/state.rs`
- Create `src/ui/painter.rs`
- Create `src/ui/linux_overlay.rs`
- Create implementation todos: `.pi/todos/sctk-wayland-layer-overlay.md`
