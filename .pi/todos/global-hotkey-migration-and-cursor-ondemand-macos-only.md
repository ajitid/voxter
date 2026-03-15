# TODOs: global-hotkey-migration-and-cursor-ondemand-macos-only

## Phase 1: Dependencies and platform gating
- [x] Remove `rdev` dependency
- [x] Add `global-hotkey` dependency
- [x] Add macOS-only compile gate in `src/main.rs`

## Phase 2: Hotkey event model migration
- [x] Remove `rdev` listener thread and related code
- [x] Add `GlobalHotKeyManager` + registered hotkeys state in `App`
- [x] Initialize hotkeys in `ApplicationHandler::resumed`
- [x] Poll `GlobalHotKeyEvent::receiver()` in `about_to_wait`

## Phase 3: Cursor positioning migration
- [x] Remove cursor tracking state from app
- [x] Change overlay API to on-demand cursor query
- [x] Add macOS cursor query function via CoreGraphics
- [x] Enforce strict cursor-monitor selection (no primary/first fallback)

## Phase 4: Integrate and clean up
- [x] Update overlay call sites and error handling
- [x] Remove dead imports and obsolete helpers

## Phase 5: Verify
- [x] `cargo fmt`
- [x] `cargo check`
- [x] `cargo clippy`
