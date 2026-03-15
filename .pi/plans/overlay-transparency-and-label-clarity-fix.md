# Plan: Fix Overlay Transparency + Garbled `recording` Text

## User-reported issues
From screenshot (`/Users/as186073/Desktop/Screenshot 2026-03-15 at 4.39.04 PM.png`):
1. Overlay background appears black instead of transparent.
2. `recording` label is hard to read / visually garbled.

User preferences confirmed:
- **Font direction:** switch to a clean, readable system sans font and **increase size by +4pt**.
- **Compatibility direction:** **no opaque fallback**; fail fast when transparent compositing is unavailable.

---

## Verified findings

### Code findings (repo)
- Overlay window is created with transparency enabled (`with_transparent(true)`) in `src/ui/overlay.rs`.
- Surface alpha mode is selected in `src/ui/render.rs` with this logic:
  - prefer `CompositeAlphaMode::PreMultiplied`,
  - otherwise fallback to `caps.alpha_modes[0]`.
- Current render pipeline blend state is `wgpu::BlendState::ALPHA_BLENDING` (non-premultiplied).
- Text is rendered with `assets/dotty.ttf` in `src/ui/render.rs`, which matches screenshot’s low-legibility pixel style.

### External references
- WGPU transparency requires non-opaque surface alpha mode and compositor support:
  - https://docs.rs/wgpu/latest/wgpu/enum.CompositeAlphaMode.html
  - https://github.com/gfx-rs/wgpu/issues/3486
- WGPU blend constants distinction (premultiplied vs non-premultiplied):
  - https://docs.rs/wgpu/latest/wgpu/struct.BlendState.html
- `font-kit` supports selecting a system sans font (`SystemSource::select_best_match`):
  - https://docs.rs/crate/font-kit/latest

---

## Root-cause hypothesis

1. **Black background:**
   - If the selected surface alpha mode is effectively opaque (or not explicitly validated), the compositor can ignore alpha and show black.
   - Also, using non-premultiplied pipeline blending with a premultiplied compositor path can produce incorrect compositing behavior.

2. **Garbled text:**
   - `dotty.ttf` is an intentionally stylized low-res font; at current scaling it harms readability.
   - Current heuristic centering/size tuning further reduces clarity.

---

## Implementation-ready patch spec

## Phase 1 — Enforce true transparency capability (fail fast)

### File: `src/ui/render.rs`

1. **Harden alpha mode selection logic in `OverlayRenderer::new(...)`:**
   - Replace current “prefer premultiplied else first mode” with explicit policy:
     - Accept only `PreMultiplied` or `PostMultiplied` (prefer `PreMultiplied`).
     - If neither exists, return `Err(...)` with a clear message listing supported alpha modes.

2. **Log selected format + alpha mode for diagnostics:**
   - Add a single `eprintln!` (or equivalent) after capability selection:
     - selected format
     - selected alpha mode
     - full supported alpha mode list

3. **Use premultiplied pipeline blending when compositor mode is premultiplied path:**
   - Replace `wgpu::BlendState::ALPHA_BLENDING` with `wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING` in `ColorTargetState`.
   - Keep transparent clear color (`a=0.0`) as-is.

> Note: This aligns with user’s “no fallback” requirement and makes unsupported systems fail immediately instead of silently rendering black.

---

## Phase 2 — Replace Dotty with readable system sans + size bump

### File: `src/ui/render.rs`

4. **Replace file-based font load (`assets/dotty.ttf`) with system sans selection:**
   - Remove:
     - `use std::fs::File;`
     - `Loader::from_file(...)` logic
   - Add imports:
     - `font_kit::family_name::FamilyName`
     - `font_kit::properties::Properties`
     - `font_kit::source::SystemSource`
   - Load font via:
     - `SystemSource::new().select_best_match(&[FamilyName::SansSerif], &Properties::new())?.load()?`
   - Return descriptive error if system font lookup/load fails.

5. **Increase text size by +4pt:**
   - Current size formula:
     - `let point_size = (self.size.height as f32 * 0.45).clamp(16.0, 42.0);`
   - Update to include +4pt and reasonable clamp increase:
     - e.g. `let point_size = ((self.size.height as f32 * 0.45) + 4.0).clamp(20.0, 46.0);`

6. **Improve readability positioning:**
   - Keep current horizontal estimate fallback but slightly adjust vertical anchor upward to avoid cramped baseline rendering.
   - Example:
     - from `y = (self.size.height as f32 * 0.62)...`
     - to `y = (self.size.height as f32 * 0.58)...`

---

## Phase 3 — Label text polish (optional but low-risk)

### File: `src/ui/overlay.rs`

7. **Title-case status strings for clarity** (if desired during implementation):
   - `recording` -> `Recording`
   - `recording (latch)` -> `Recording (Latch)`
   - `transcribing` -> `Transcribing`

(If unchanged, readability still improves from font + size changes.)

---

## Phase 4 — Validation checklist

### Static checks
- `cargo fmt`
- `cargo check`
- `cargo clippy`

### Runtime verification (macOS)
1. Start HOLD recording.
   - Expect: overlay shows transparent background with only readable white text.
2. Switch to latch.
   - Expect: label updates, remains sharp/readable.
3. Stop recording / transcribing states.
   - Expect: same transparency behavior throughout.
4. Confirm failure behavior on unsupported alpha mode:
   - App should return clear initialization error rather than rendering opaque black.

### Acceptance criteria
- No black rectangle background under overlay text.
- `recording` text is clearly legible at normal viewing distance.
- Initialization clearly reports unsupported transparency capability instead of silent fallback.

---

## Exact edit points summary
- `src/ui/render.rs`
  - `OverlayRenderer::new(...)` capability/alpha selection block
  - pipeline `ColorTargetState.blend`
  - font loading block near current `assets/dotty.ttf` load
  - `draw_label(...)` point size + y positioning constants
- `src/ui/overlay.rs` (optional)
  - state string mappings in `update_state(...)` and `handle_resize(...)`

---

## Notes
- `assets/dotty.ttf` may be retained in repo for history, but it is no longer used by overlay rendering after this patch.
- This plan intentionally does **not** include an opaque fallback path per user direction.
