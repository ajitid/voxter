# Plan: Engine-style overlay draw loop (active only when visible)

## Goal
Add a continuous, game-engine-style render loop for the overlay **only while visible** (Recording / RecordingLatch / Transcribing), and ensure the app returns to idle waiting when overlay is hidden.

This enables richer `raqote` drawing/animation without burning CPU when idle.

## Context and findings

### Current repo behavior
- Event loop is currently passive:
  - `src/main.rs:929` sets `ControlFlow::Wait`.
  - `src/main.rs:873` `about_to_wait()` is empty.
- Redraw is one-shot on state changes:
  - `src/ui/overlay.rs:100` calls `window.request_redraw()` in `update_state`.
  - `src/main.rs:863-866` handles `WindowEvent::RedrawRequested` by calling `overlay.redraw()`.
- `raqote` draw entry point is label-only:
  - `src/ui/render.rs:363` `draw_label(&mut self, text: &str)`.

### Engine loop references (why this model)
- `winit` docs/source: `ControlFlow::Poll` is the continuous mode; `WaitUntil` is timer-based. Source comment notes apps targeting native refresh should prefer `Poll` + graphics API VSync/present pacing.
  - Local reference: `~/.cargo/registry/src/.../winit-0.30.13/src/event_loop.rs` around `ControlFlow` docs.
- Bevy (`UpdateMode`) separates **Continuous** vs **Reactive** updates and explicitly treats this as independent from VSync.
  - https://docs.rs/bevy/latest/bevy/winit/enum.UpdateMode.html
- Defold supports active loop plus engine throttling/skip updates when idle.
  - https://defold.com/manuals/application-lifecycle/
- Unity runs continuously when active, with background pause behavior (`runInBackground`).
  - https://docs.unity3d.com/ScriptReference/Application-runInBackground.html
- Classic timing article: fixed sleep is not the only/primary game-loop approach; continuous + present pacing is common.
  - https://gafferongames.com/post/fix_your_timestep/

## User decisions locked
- Loop model: **Engine-style active loop**
  - `ControlFlow::Poll + request_redraw()` while overlay visible.
  - `ControlFlow::Wait` while overlay hidden.
- API refactor allowed: **Yes** (small internal breaking change for cleaner animation API).

## Implementation design

### 1) Introduce explicit overlay visibility/animation state API
**File:** `src/ui/overlay.rs`

#### Add methods
- `pub fn is_visible(&self) -> bool` (true when `self.state != OverlayState::Hidden`).
- `pub fn current_state(&self) -> OverlayState` (optional helper, useful for renderer calls).

#### Refactor redraw API
- Change `pub fn redraw(&mut self) -> Result<(), String>` to render per-frame content via renderer’s frame API (see section 2).
- Keep state transitions in `update_state()`; do not pre-render static label there anymore.
  - It should:
    - set `self.state`
    - hide window if hidden
    - position + show window if visible
    - request first redraw when entering visible state

#### Resize handling
- `handle_resize()` should only resize resources and request redraw if visible.
- Remove static `draw_label()` dependency.

---

### 2) Replace label-only renderer with per-frame renderer API
**File:** `src/ui/render.rs`

#### New API
- Replace `draw_label(&mut self, text: &str)` with:
  - `pub fn draw_frame(&mut self, state: OverlayState, now: Instant, started_at: Instant)`
    - or equivalent compact signature that includes state + timing.
- Keep `render(&mut self)` unchanged as GPU present pass.

#### Internal structure changes
- Add lightweight animation timing inputs and derive `t` (seconds since visible-start).
- Add helper(s):
  - `fn state_label(state: OverlayState) -> &'static str`
  - `fn draw_text(...)` (extracted from old glyph layout path)
  - optional helpers for background/badge visuals.

#### Draw behavior
- Every frame:
  - clear transparent target
  - draw background shape(s) and animated accents with `raqote`
  - draw label text based on state
  - upload `DrawTarget` bytes to texture (existing `queue.write_texture` path)

This gives a stable per-frame hook for “draw more using raqote”.

---

### 3) Drive loop mode from overlay visibility
**File:** `src/main.rs`

#### Add loop control helper in `App`
- `fn update_loop_mode(&self, event_loop: &ActiveEventLoop)`
  - if overlay exists and `overlay.is_visible()` -> `event_loop.set_control_flow(ControlFlow::Poll)`
  - else -> `event_loop.set_control_flow(ControlFlow::Wait)`

#### Call points
- In `resumed()`: after overlay creation, set loop mode (should remain Wait because hidden).
- In `user_event()` after `AppEvent::Overlay(state)` processed: call `update_loop_mode(event_loop)`.
- In `window_event()` on `CloseRequested` / hidden transitions as needed.

#### Continuous redraw trigger (visible only)
- In `about_to_wait()`:
  - if overlay visible -> `overlay.request_redraw()` (new forwarding method in `OverlayController`, or expose window redraw request via method).
  - else do nothing.

#### Redraw handling
- `WindowEvent::RedrawRequested`:
  - call `overlay.redraw()` once per event.
  - do not redraw when hidden (guard inside overlay).

This produces: Poll loop + redraw requests while visible; Wait with no redraw requests while hidden.

---

### 4) Keep hidden path zero-work
**Files:** `src/ui/overlay.rs`, `src/main.rs`

- On transition to `OverlayState::Hidden`:
  - `window.set_visible(false)`
  - app loop mode switches to `ControlFlow::Wait`
  - no per-frame redraw requests from `about_to_wait()` due to visibility guard

Net effect: no continuous loop work when not recording/transcribing.

## Patch-spec (exact edit points)

1. `src/ui/overlay.rs`
   - Edit struct `OverlayController`:
     - add timing field for visible-session start (e.g., `visible_since: Option<Instant>`).
   - Edit `update_state(...)`:
     - remove direct `renderer.draw_label(label)` path.
     - set/clear `visible_since`, show/hide window.
     - request redraw on visible entry.
   - Edit `handle_resize(...)`:
     - remove old `draw_label` calls.
     - request redraw only when visible.
   - Edit `redraw(...)`:
     - pass state/time to renderer frame API.
   - Add methods:
     - `is_visible()`
     - `request_redraw()` wrapper.

2. `src/ui/render.rs`
   - Replace `draw_label(&mut self, text: &str)` with `draw_frame(...)`.
   - Extract old text shaping from `draw_label` into helper used by `draw_frame`.
   - Keep texture upload + `render()` logic intact.
   - Import overlay state type (`use crate::ui::overlay::OverlayState;`) and timing (`std::time::Instant`).

3. `src/main.rs`
   - Add `App::update_loop_mode(&self, event_loop: &ActiveEventLoop)`.
   - After overlay state updates in `user_event`, call `update_loop_mode`.
   - Implement `about_to_wait` to request redraw when visible.
   - Keep initial `event_loop.set_control_flow(ControlFlow::Wait)`.

## Verification plan

1. Build/lint
- `cargo check`
- `cargo fmt`
- `cargo clippy`

2. Runtime behavior checks
- Start app idle (hidden): confirm low CPU and no redraw spam logs.
- Hold record (visible): confirm smooth continuous redraw and animations.
- Release / transcribe / hidden transitions: confirm redraw continues only while visible.
- Resize event handling: no panics, redraw still works.

3. Guardrails
- Ensure hidden state always drives `ControlFlow::Wait`.
- Ensure no code path calls `request_redraw()` when hidden.

## Notes / risk
- `Poll` can still be high-frequency on some setups if present pacing is not effective; if this appears in practice, add optional cap fallback (feature flag/env) to `WaitUntil(16ms)` while visible.
- This plan intentionally keeps that fallback out of default path to match selected engine-style behavior.
