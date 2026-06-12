# GNOME tray icon color fix

## Phase 1: Diagnose
- [x] Inspect screenshot and installed tray SVG
- [x] Confirm GNOME Wayland + AppIndicator extension
- [x] Identify hard-coded dark symbolic SVG as likely cause

## Phase 2: Implement
- [x] Add GNOME-specific light in-memory SNI pixmap
- [x] Keep existing icon-name path for KDE/other desktops

## Phase 3: Verify
- [x] Run cargo fmt
- [x] Run cargo check
