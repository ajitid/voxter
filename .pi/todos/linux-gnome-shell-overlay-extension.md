# Linux GNOME Shell overlay extension implementation

## Extension files
- [x] Add GNOME Shell extension metadata, stylesheet, and D-Bus/drawing implementation.

## Install script
- [x] Add executable install helper script.

## Rust platform split
- [x] Split overlay module into shared public module plus macOS winit/wgpu controller.
- [x] Gate winit window event handling to macOS.

## Linux D-Bus overlay controller
- [x] Add Linux controller using session D-Bus to talk to the Shell extension.
- [x] Add startup availability check.

## Startup hard requirement
- [x] Fail startup on Linux/GNOME Wayland if extension Ping fails before hotkey registration.

## Docs
- [x] Add Linux GNOME Shell overlay docs.
- [x] Update Linux workaround docs and CLAUDE.md.

## Verification
- [x] Run cargo fmt.
- [x] Run cargo check.
- [x] Run cargo clippy.
- [ ] Manual GNOME extension install/busctl/app behavior checks.
