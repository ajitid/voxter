# TODO: overlay-transparency-and-label-clarity-fix

## Phase 1 — Transparency mode hardening
- [x] Enforce PreMultiplied/PostMultiplied alpha mode selection in `src/ui/render.rs`
- [x] Fail fast with descriptive error when transparent compositing is unsupported
- [x] Add diagnostics log for selected format/alpha/supported alpha modes
- [x] Align pipeline blend state with selected compositor alpha mode

## Phase 2 — Font readability improvements
- [x] Replace `assets/dotty.ttf` loading with system sans font loading via `font-kit::SystemSource`
- [x] Increase overlay font size by +4 points (with adjusted clamp)
- [x] Slightly tune vertical placement for clearer readability

## Phase 3 — Validation
- [x] Run `cargo fmt`
- [x] Run `cargo check`
- [x] Run `cargo clippy`
