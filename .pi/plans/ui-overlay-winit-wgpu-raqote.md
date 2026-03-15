# Plan: Recording/Transcribing Overlay UI (winit + wgpu + raqote)

## Goal
Implement a transparent, click-through, always-on-top overlay window that:
- Appears when recording starts.
- Shows `recording` in HOLD mode.
- Shows `recording (latch)` in LATCH mode.
- Shows `transcribing` during API transcription.
- Hides when transcription is complete (or recording is skipped/no speech).
- Appears on the monitor containing the cursor, positioned bottom-center.
- Uses `assets/dotty.ttf` for text rendering.

---

## Verified findings (repo + web)

### Current project state
- Single-file app in `src/main.rs` with blocking main loop and rdev hotkey thread.
- Recording/transcription lifecycle already exists in `AudioManager` and background worker threads.
- Existing dependencies are audio/network/input focused; no UI stack yet.

### Library versions (current latest)
- `raqote = 0.8.5` (`cargo search/info`)
- `wgpu = 28.0.0` (`cargo info`)
- `winit = 0.31.0-beta.2` latest pre-release; `0.30.12` latest stable

### User-confirmed version policy and behavior
- Use **latest stable winit** (`0.30.x`) rather than beta.
- OK to do **breaking internal architecture refactor** to event-loop model.
- Overlay should be **always-on-top** and **click-through**.
- Click-through scope: **macOS-first guaranteed**, best-effort/no-op elsewhere.

### raqote text support check
- `raqote` supports text rendering directly via `DrawTarget::draw_text(...)`.
- It can use file-loaded fonts (`font-kit` loader), so `assets/dotty.ttf` is usable.
- Therefore no alternative text-rendering stack is required.

References:
- `src/main.rs`
- `Cargo.toml`
- https://docs.rs/raqote/latest/raqote/struct.DrawTarget.html (draw_text)
- https://docs.rs/winit/0.30.12/winit/

---

## Breaking changes (explicit)
1. **Main control flow refactor**:
   - Replace current blocking `mpsc.recv()` loop in `main` with a `winit` event loop app.
   - Hotkey and worker thread events will be forwarded into `winit` via `EventLoopProxy<UserEvent>`.
2. **UI lifecycle integrated into app state**:
   - Recording/transcription state transitions now trigger overlay state updates.

User already approved this breaking internal architecture change.

---

## Implementation-ready patch spec

## Phase 1 — Dependencies and module scaffolding

### 1. Edit `Cargo.toml`
Add UI/render dependencies:
- `winit = "0.30.12"`
- `wgpu = "28.0.0"`
- `pollster = "0.4"` (for simple async wgpu init blocking)
- `raqote = "0.8.5"`
- `font-kit = "0.14"` (for loading `assets/dotty.ttf` into raqote text API)
- `bytemuck = { version = "1", features = ["derive"] }` (vertex/ubo casting for wgpu)

> Keep existing audio/transcription deps unchanged.

### 2. Add new UI modules
Create files:
- `src/ui/mod.rs`
- `src/ui/overlay.rs`
- `src/ui/render.rs`

Responsibilities:
- `overlay.rs`: window creation, positioning (cursor monitor bottom-center), visibility/state updates.
- `render.rs`: wgpu setup + textured quad renderer + texture uploads from raqote output.

---

## Phase 2 — App event model and main-loop refactor

### 3. Refactor `src/main.rs` into app/event-loop structure

#### Exact edit points
- Replace current `main()` loop section (from `// Control channel from hotkey listener -> main thread` onward) with winit-driven app handler.
- Keep audio pipeline functions (`AudioManager`, `run_opus_worker`, VAD, API transcription) but adapt control entry points.

#### Introduce new enums (in `src/main.rs`)
- `enum HotkeyMsg { StopHold, SinglePress, SwitchToLatch, TypeLastTranscription, Quit }`
- `enum UiState { Hidden, RecordingHold, RecordingLatch, Transcribing }`
- `enum AppEvent { Hotkey(HotkeyMsg), Ui(UiState), TranscriptionDone }`

#### New app state struct
- `struct App {`
  - `audio_manager: AudioManager,`
  - `overlay: ui::overlay::OverlayController,`
  - `last_cursor_pos: Arc<Mutex<(f64, f64)>>,`
  - `proxy: EventLoopProxy<AppEvent>,`
`}`

#### rdev listener changes
- In hotkey thread, send `AppEvent::Hotkey(...)` through `EventLoopProxy`.
- Also track `MouseMove { x, y }` from rdev events into `last_cursor_pos` so monitor selection is accurate even when overlay window is click-through.

---

## Phase 3 — Overlay window behavior

### 4. Implement `OverlayController` in `src/ui/overlay.rs`

#### Public API
- `fn new(event_loop: &ActiveEventLoop) -> Result<Self, String>`
- `fn set_state(&mut self, state: UiState, cursor_pos: (f64, f64), event_loop: &ActiveEventLoop) -> Result<(), String>`
- `fn request_redraw(&self)`
- `fn handle_redraw(&mut self) -> Result<(), String>`

#### Window creation attributes
- Transparent: `with_transparent(true)`
- No decorations: `with_decorations(false)`
- Initially hidden: `with_visible(false)`
- Not resizable: `with_resizable(false)`
- Always-on-top: `with_window_level(WindowLevel::AlwaysOnTop)`
- Small fixed surface size (e.g. 420x96 logical)

#### Click-through
- macOS: use `winit::platform::macos::WindowExtMacOS` to ignore mouse events.
- non-macOS: no-op with log note.

#### Positioning logic
- On show/update:
  - find monitor containing cursor (using monitor position+size rectangles).
  - fallback to primary monitor if none matches.
  - set outer position to bottom-center with margin (e.g. 36 px from bottom).

---

## Phase 4 — Rendering with raqote -> wgpu texture

### 5. Implement renderer in `src/ui/render.rs`

#### Pipeline
1. Maintain an RGBA/BGRA CPU pixel buffer from `raqote::DrawTarget`.
2. Render text to transparent background with `raqote`.
3. Upload buffer to a `wgpu` texture each state change.
4. Render full-window textured quad in redraw event.

#### Text rendering details
- Load font once from `assets/dotty.ttf` via `font_kit::loader::Loader::from_file`.
- Text mapping:
  - `RecordingHold` -> `recording`
  - `RecordingLatch` -> `recording (latch)`
  - `Transcribing` -> `transcribing`
- White text, transparent background.
- Center text horizontally/vertically using measured glyph bounds (or conservative centering approximation if bounds are unavailable from API).

#### wgpu surface config
- Configure with alpha-capable format and transparent clear color.
- Handle resize/surface-lost events by reconfiguring surface.

---

## Phase 5 — Wire state transitions into recording/transcription lifecycle

### 6. In `AudioManager` methods (in `src/main.rs`)

#### `start_recording(...)`
- after successful stream start:
  - send `AppEvent::Ui(UiState::RecordingHold)`

#### `switch_to_latch_mode(...)`
- on successful switch:
  - send `AppEvent::Ui(UiState::RecordingLatch)`

#### `stop_recording(...)`
- when finalizing and before API call thread:
  - send `AppEvent::Ui(UiState::Transcribing)` if transcription is expected.
- if duration too short / no speech:
  - send `AppEvent::Ui(UiState::Hidden)` immediately.
- after transcription thread completes (success or error):
  - send `AppEvent::Ui(UiState::Hidden)`.

Implementation note:
- Threaded functions that currently don’t know app context will receive a cloned `EventLoopProxy<AppEvent>` (or callback wrapper) to publish UI events safely.

---

## Phase 6 — Validation and quality checks

### 7. Build/test steps
- `cargo fmt`
- `cargo check`
- `cargo clippy`
- Manual runtime checks (macOS):
  1. HOLD key down: overlay appears bottom-center on cursor monitor with `recording`.
  2. Press space during hold: label updates to `recording (latch)`.
  3. Stop recording: label changes to `transcribing`.
  4. On completion: overlay disappears.
  5. Move cursor to another monitor and repeat: overlay appears on that monitor.
  6. Confirm overlay is click-through (cannot steal/consume clicks).

---

## Risks and mitigations
- **winit API churn between minor versions**: pin to `0.30.12` (user requested stable latest).
- **Transparent surface quirks per platform**: macOS-first guarantee; keep non-mac as best-effort.
- **Threaded state update races**: centralize UI state transitions through `AppEvent` queue.
- **Potential text centering mismatch** with custom font metrics: add small tuneable offsets/constants.

---

## Deliverables after implementation
- Overlay UI integrated with recording lifecycle.
- New UI modules under `src/ui/`.
- Updated dependencies in `Cargo.toml`.
- Maintained existing transcription typing behavior.
- macOS click-through always-on-top transparent overlay with dotty font text.
