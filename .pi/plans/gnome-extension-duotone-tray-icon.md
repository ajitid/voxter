# Plan: Use macOS duotone tray icon artwork in GNOME panel

## Goal

Replace the GNOME extension panel icon currently using `audio-input-microphone-symbolic` with artwork matching the existing macOS status/menu-bar icon from `src/ui/tray.rs`.

Important user preference: use a **theme-aware symbolic SVG with two opacity levels**. The icon should preserve the macOS duotone structure, but inherit GNOME panel foreground color instead of hardcoding black.

## References / current facts

- macOS status icon is generated in Rust:
  - `src/ui/tray.rs`
  - function: `build_status_icon()`
- GNOME extension panel icon is created in:
  - `gnome-shell-extension/voxtral-speech-to-text@ajitid/extension.js`
  - current code uses:
    ```js
    this._indicatorIcon = new St.Icon({
        icon_name: 'audio-input-microphone-symbolic',
        style_class: 'system-status-icon',
    });
    ```
- GNOME extension stylesheet exists:
  - `gnome-shell-extension/voxtral-speech-to-text@ajitid/stylesheet.css`
- Existing macOS artwork is duotone via alpha:
  - primary strokes: alpha `255`
  - secondary/muted strokes: alpha `145`
- Existing macOS icon geometry:
  - canvas: `64x64`
  - viewbox: `22x22`
  - path/stroke coordinates are in `src/ui/tray.rs`

## Design

Create a static SVG asset in the extension directory that encodes the same path geometry as the Rust-generated icon.

Use `currentColor` plus opacity to make it GNOME theme-aware:

- primary strokes: `stroke="currentColor"`, `stroke-opacity="1"`
- muted strokes: `stroke="currentColor"`, `stroke-opacity="0.568627"` (`145 / 255`)
- no hardcoded black
- transparent background
- rounded stroke caps/joins
- viewBox `0 0 22 22`

Then load it with `Gio.FileIcon` through the extension object, not through the icon theme. This avoids needing system icon-theme installation.

## Patch spec

### A. Add SVG asset

Create:

```text
gnome-shell-extension/voxtral-speech-to-text@ajitid/voxtral-status-symbolic.svg
```

Recommended SVG content:

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 22 22">
  <g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round">
    <path d="M4.6 10.8H17.4" stroke-width="1.6"/>
    <path d="M5.8 6.1L4.7 10.8L5.8 15.5M16.2 6.1L17.3 10.8L16.2 15.5" stroke-width="1.6"/>
    <path d="M6.7 7L5.8 5.6M15.3 7L16.2 5.6M6.7 14.6L5.8 16M15.3 14.6L16.2 16" stroke-width="1.15" stroke-opacity="0.568627"/>
    <path d="M6 6.3L11 10.8L16 6.3M6 15.3L11 10.8L16 15.3M7.8 6.2L14.2 15.4M14.2 6.2L7.8 15.4" stroke-width="1.15" stroke-opacity="0.568627"/>
  </g>
</svg>
```

This is a direct translation of the `PathBuilder` commands in `src/ui/tray.rs` from raqote to SVG path data.

### B. Update GNOME extension icon loading

In `gnome-shell-extension/voxtral-speech-to-text@ajitid/extension.js`, replace the current icon construction:

```js
this._indicatorIcon = new St.Icon({
    icon_name: 'audio-input-microphone-symbolic',
    style_class: 'system-status-icon',
});
```

with:

```js
const iconFile = this.dir.get_child('voxtral-status-symbolic.svg');
this._indicatorIcon = new St.Icon({
    gicon: new Gio.FileIcon({file: iconFile}),
    style_class: 'system-status-icon voxtral-status-icon',
});
```

Notes:

- `this.dir` is available on GNOME Shell `Extension` instances and resolves to the extension directory.
- `Gio` is already imported in `extension.js`.
- Keep the existing `system-status-icon` class so the icon receives GNOME panel sizing/color behavior.
- Add `voxtral-status-icon` only if stylesheet tuning is needed.

### C. Optional stylesheet tuning

Check `gnome-shell-extension/voxtral-speech-to-text@ajitid/stylesheet.css`.

If the loaded SVG does not size exactly like a system icon, add:

```css
.voxtral-status-icon {
    icon-size: 16px;
}
```

Do not hardcode color unless runtime testing shows `currentColor` is not inherited for file-backed SVG icons. If color inheritance fails, investigate GNOME Shell symbolic SVG recoloring before falling back to hardcoded white/black.

### D. Keep no fallback by default

Do not keep `audio-input-microphone-symbolic` fallback unless implementation discovers a real GNOME Shell version compatibility issue. User generally prefers fail-fast over hidden fallback behavior, and the asset is bundled with the extension.

If a fallback is added after testing, document why in code with the exact GNOME Shell error or version issue.

### E. Update docs/messages only if useful

No install-script message change is required. The existing extension pack command will include the SVG asset automatically because it packs the whole extension directory.

Optionally mention the custom panel icon in:

- `docs/linux-gnome-shell-overlay.md`

Only do this if that doc already describes panel menu behavior after the previous tray-icon plan.

## Verification plan

### Static checks

Run:

```sh
cargo fmt
cargo check
```

Even though this is mostly extension work, run these to ensure no Rust regressions if shared code/docs were touched.

Check that the SVG is included in extension source:

```sh
ls -l gnome-shell-extension/voxtral-speech-to-text@ajitid/voxtral-status-symbolic.svg
```

Check for unwanted fallback/system icon use:

```sh
rg -n "audio-input-microphone-symbolic|voxtral-status-symbolic|FileIcon|gicon" gnome-shell-extension/voxtral-speech-to-text@ajitid
```

Expected:

- `audio-input-microphone-symbolic` absent unless an explicitly justified fallback was added.
- `voxtral-status-symbolic.svg` referenced from `extension.js`.

### SVG sanity checks

If available, run one or more of:

```sh
xmllint --noout gnome-shell-extension/voxtral-speech-to-text@ajitid/voxtral-status-symbolic.svg
rsvg-convert gnome-shell-extension/voxtral-speech-to-text@ajitid/voxtral-status-symbolic.svg -o /tmp/voxtral-status.png
```

If `xmllint`/`rsvg-convert` are not installed, do not add project dependencies just for this; manually inspect XML and rely on GNOME runtime verification.

### Runtime GNOME verification

Reinstall extension:

```sh
scripts/install-gnome-shell-extension.sh
```

Then log out/in if the running extension was already loaded.

Verify:

```sh
gnome-extensions info voxtral-speech-to-text@ajitid
```

Run app:

```sh
cargo run
```

Expected:

- GNOME top-panel icon uses the same shock-mount/studio silhouette as macOS.
- Icon color follows panel/theme color.
- Primary geometry is fully opaque; secondary geometry appears muted/duotone.
- Menu behavior remains unchanged:
  - `Type last transcript` disabled until a transcript exists.
  - `Quit` enabled while app runs.

## Risks / notes

- GNOME Shell symbolic SVG handling may not apply `currentColor` identically for file-backed `Gio.FileIcon` on all shell versions. If testing shows incorrect color, investigate whether GNOME requires symbolic icon naming or CSS properties for `St.Icon` before hardcoding colors.
- The macOS icon is currently rasterized from vector commands at runtime. This plan intentionally duplicates geometry in SVG. If the macOS icon changes later, update the SVG in the same commit to keep platforms visually aligned.
- Because this is theme-aware, it will not be pixel-identical to macOS black alpha output on every theme; it should be visually equivalent and integrate better with GNOME.

## Assumptions

- User wants the GNOME icon to fail visibly if the bundled asset cannot load, not silently fall back to the generic microphone icon.
- No GTK/AppIndicator/tray-icon Linux dependencies should be introduced.
- The SVG asset should be bundled in the GNOME extension, not generated by Rust at runtime.
