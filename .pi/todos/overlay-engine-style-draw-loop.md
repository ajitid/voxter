# TODO: overlay-engine-style-draw-loop

## Phase 1 — Overlay controller loop-facing API
- [x] Add visibility/timing state to `src/ui/overlay.rs`
- [x] Refactor `update_state()` to handle show/hide + first redraw only
- [x] Refactor `handle_resize()` and `redraw()` for frame-driven rendering
- [x] Add `is_visible()` and `request_redraw()` methods

## Phase 2 — Renderer frame API
- [x] Replace label-only renderer API with `draw_frame(state, now, started_at)` in `src/ui/render.rs`
- [x] Extract state label + text draw helpers
- [x] Add simple per-frame animated accents and keep texture upload path intact

## Phase 3 — Event loop integration
- [x] Add `App::update_loop_mode()` in `src/main.rs`
- [x] Switch control flow between `Poll` (visible) and `Wait` (hidden)
- [x] Trigger redraw from `about_to_wait()` only when overlay is visible

## Phase 4 — Validation
- [x] Run `cargo fmt`
- [x] Run `cargo check`
- [x] Run `cargo clippy`
