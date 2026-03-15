# TODOs: rdev-github-main-option-b

## Phase 1: Dependency switch
- [x] Replace `global-hotkey` with `rdev` from GitHub main

## Phase 2: Listener migration
- [x] Remove `global-hotkey` manager/event polling wiring
- [x] Restore `rdev` listener thread
- [x] Call `rdev::set_is_main_thread(false)` before `listen`
- [x] Keep Right Cmd HOLD / Right Cmd + Quote / Space semantics

## Phase 3: Verify
- [x] cargo fmt
- [x] cargo check
- [x] cargo clippy
