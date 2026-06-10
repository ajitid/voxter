# TODO: privileged Linux hotkey helper

## Phase 1: Cargo + helper
- [x] Rename Cargo package and add bin targets
- [x] Add `voxter-hotkey-helper` binary

## Phase 2: Main app integration
- [x] Split hotkey listener by platform
- [x] Add Linux pkexec/stdout JSON-lines handling

## Phase 3: Packaging/docs
- [x] Add polkit policy
- [x] Add Linux helper install script
- [x] Add Linux Wayland helper docs

## Phase 4: Verify
- [x] Run `cargo fmt`
- [x] Run `cargo check --bin voxter`
- [x] Run `cargo check --bin voxter-hotkey-helper`
- [x] Run `cargo clippy --all-targets`
