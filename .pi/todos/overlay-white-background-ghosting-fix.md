# TODO: overlay-white-background-ghosting-fix

## Phase 1 — Diagnose compositing path
- [x] Confirm runtime surface alpha mode and supported modes on macOS
- [x] Verify whether current pipeline applies alpha more than once for PostMultiplied mode
- [x] Reproduce whether old text persists in the offscreen DrawTarget buffer across state changes
- [x] Identify macOS transparent-window shadow as likely source of perceived “ghost” text

## Phase 2 — Implement fix
- [x] Prevent double alpha application in overlay render pipeline
- [x] Ensure fragment output matches compositor expectation for selected alpha mode
- [x] Keep transparent clear behavior intact
- [x] Disable macOS window shadow for the transparent overlay window

## Phase 3 — Validate
- [x] Run `cargo fmt`
- [x] Run `cargo check`
- [x] Run `cargo clippy`
