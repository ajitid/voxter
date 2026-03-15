# Plan: Migrate from `rdev` to `global-hotkey` + on-demand cursor query (macOS-only, no fallback)

## Goal
Replace `rdev` entirely with `global-hotkey` and replace mouse-move cursor tracking with on-demand cursor position lookup when showing/updating the overlay.

### Explicit decisions locked
- **No backward compatibility with `rdev`**.
- **No fallback behavior**.
- **Platform scope:** **macOS-only** for this implementation; non-macOS builds must fail explicitly.

## Why this plan
- Crash reports show SIGTRAP in `rdev` macOS keyboard translation path (`TSMGetInputSourceProperty` via `rdev::macos::keyboard::Keyboard::string_from_code`) on the hotkey listener thread.
- `rdev` latest crates.io release is `0.5.3`; newer fixes are only on unreleased git main.
- `global-hotkey` is actively maintained and does not use `rdev` on macOS.

## References
- Crash report path: `~/Library/Logs/DiagnosticReports/voxtral-speech-to-text-2026-03-15-155842.ips`
- `rdev` usage in repo: `src/main.rs` (`spawn_hotkey_listener`, `rdev::listen`)
- `global-hotkey` docs: https://docs.rs/global-hotkey/latest/global_hotkey/
- `GlobalHotKeyEvent` includes pressed/released states: https://docs.rs/global-hotkey/latest/global_hotkey/struct.GlobalHotKeyEvent.html
- crates metadata:
  - `rdev` latest crates.io: `0.5.3`
  - `global-hotkey` latest crates.io: `0.7.0`

---

## Patch spec (implementation-ready)

### 1) Dependencies + platform gate

#### Edit: `Cargo.toml`
1. Remove:
   - `rdev = "0.5"`
2. Add:
   - `global-hotkey = "0.7.0"`
3. Add explicit platform gate for build:
   - At top of `src/main.rs`, add:
     ```rust
     #[cfg(not(target_os = "macos"))]
     compile_error!("This build currently supports macOS only (global-hotkey + on-demand cursor query).");
     ```

---

### 2) Event model refactor (remove `rdev` thread path)

#### Edit: `src/main.rs`

##### 2.1 Remove old listener infrastructure
Remove:
- `spawn_hotkey_listener(...)`
- `cursor_pos: Arc<Mutex<(f64, f64)>>` from `App`
- `App::current_cursor_pos()`
- all `EventType::MouseMove`-driven cursor tracking code

##### 2.2 Introduce global-hotkey manager wiring
Add imports:
```rust
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
```

Add app-owned registrations:
```rust
struct RegisteredHotkeys {
    hold: HotKey,
    quote_combo: HotKey,
    latch_toggle: HotKey,
}
```

Add field on `App`:
```rust
hotkeys: Option<(GlobalHotKeyManager, RegisteredHotkeys)>,
```

##### 2.3 Initialize hotkeys in `ApplicationHandler::resumed`
In `resumed`:
1. Create manager on main thread:
   ```rust
   let manager = GlobalHotKeyManager::new()?;
   ```
2. Register keys (physical codes):
   - hold key: `Code::MetaRight`
   - quote combo: `Code::Quote` + `Modifiers::META`
   - latch key: `Code::Space`
3. Store manager + hotkey IDs in `self.hotkeys`.

##### 2.4 Poll global-hotkey events in event loop
In `about_to_wait`:
- Drain `GlobalHotKeyEvent::receiver().try_recv()` in loop.
- Translate by `event.id` + `event.state`:
  - hold (`MetaRight`):
    - `Pressed` => `ControlMsg::SinglePress`
    - `Released` => `ControlMsg::StopHold`
  - quote combo:
    - `Released` => `ControlMsg::TypeLastTranscription`
  - space:
    - `Pressed` => `ControlMsg::SwitchToLatch`
- Send through `self.proxy.send_event(AppEvent::Control(...))`.

##### 2.5 Remove old bootstrap calls from `main()`
Remove:
- `cursor_pos` allocation
- `spawn_hotkey_listener(...)`
- `App::new(proxy, cursor_pos)`

Replace with:
- `App::new(proxy)`

---

### 3) Cursor position on-demand (no tracking)

#### Edit: `src/ui/overlay.rs`

##### 3.1 Change API to fetch cursor internally
Change:
```rust
pub fn update_state(&mut self, event_loop: &ActiveEventLoop, next_state: OverlayState, cursor_pos: (f64, f64))
```
To:
```rust
pub fn update_state(&mut self, event_loop: &ActiveEventLoop, next_state: OverlayState)
```

Inside `update_state`, before positioning, call:
```rust
let cursor_pos = current_cursor_position()?; // or Result handling inline
```

##### 3.2 Add macOS-only cursor query function
Add (macOS only) function using CoreGraphics:
- call `CGEvent::new(None)` + `location()` (or equivalent safe binding) to get global cursor coordinates.
- return `(f64, f64)`.

If cursor query fails, return error and **do not silently fallback**.

##### 3.3 Remove fallback monitor selection
Current logic uses:
```rust
find_monitor_for_cursor(...)
    .or_else(primary_monitor)
    .or_else(first_available)
```

Replace with strict behavior:
- Must resolve monitor via cursor containment only.
- If none found, return error and keep overlay hidden (no fallback to primary/first).

---

### 4) Wire overlay call sites to new API

#### Edit: `src/main.rs`
Update call in `user_event`:
```rust
overlay.update_state(event_loop, state);
```

Handle `Result`/error from overlay update by logging and continuing (no fallback positioning path).

---

### 5) Remove dead imports/struct fields

#### Edit: `src/main.rs`
Clean up:
- remove `Arc<Mutex<(f64, f64)>>` fields/import usage tied to cursor tracking
- remove `rdev`-related imports and code paths

#### Edit: `src/ui/overlay.rs`
Clean up:
- remove now-unused cursor-parameter plumbing
- ensure monitor-selection helpers align with strict/no-fallback policy

---

## Validation plan
1. `cargo fmt`
2. `cargo check`
3. `cargo clippy`
4. Manual macOS runtime checks:
   - Press/release Right Cmd: overlay shows `recording`, then short-record skip path hides cleanly.
   - Press Space during hold: switches to `recording (latch)`.
   - Press Right Cmd again in latch: transitions to `transcribing`, then hidden.
   - Right Cmd + Quote retypes last transcription.
   - Verify no SIGTRAP after repeated tap/release sequences.
   - Verify overlay appears on cursor’s monitor bottom-center without moving mouse first.

## Risks / watchpoints
- `global-hotkey` registration semantics for pure modifier keys (`MetaRight`) can vary; verify empirically.
- If pure `MetaRight` cannot be registered reliably, this plan must switch trigger key design (would require new product decision).
- Strict no-fallback positioning means overlay may intentionally not appear if cursor/monitor resolution fails.

## Out of scope
- Non-macOS support restoration.
- Any `rdev` fallback path.
- Alternate hotkey schema changes unless registration proves impossible.
