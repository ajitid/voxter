# TODOs: ui-overlay-winit-wgpu-raqote

## Phase 1: Dependencies and scaffolding
- [x] Update `Cargo.toml` with `winit`, `wgpu`, `pollster`, `raqote`, `font-kit`, `bytemuck`
- [x] Add `src/ui/mod.rs`
- [x] Add `src/ui/overlay.rs`
- [x] Add `src/ui/render.rs`

## Phase 2: Main event loop refactor
- [x] Replace blocking main loop with winit `ApplicationHandler` loop
- [x] Add app user events and event proxy wiring
- [x] Keep hotkeys via `rdev` thread -> proxy user events

## Phase 3: Overlay behavior
- [x] Transparent, undecorated, always-on-top window
- [x] Cursor-monitor bottom-center positioning
- [x] Visibility/text state transitions
- [x] macOS click-through support

## Phase 4: Rendering pipeline
- [x] Initialize wgpu surface/device/queue
- [x] Render textured quad with alpha
- [x] Draw text via raqote using `assets/dotty.ttf`
- [x] Upload raqote buffer to wgpu texture on state changes

## Phase 5: Integrate audio lifecycle
- [x] Show `recording` when hold starts
- [x] Show `recording (latch)` when latch toggles
- [x] Show `transcribing` during API work
- [x] Hide overlay when done / no-speech / short recording

## Phase 6: Verify
- [x] `cargo fmt`
- [x] `cargo check`
- [x] `cargo clippy`
