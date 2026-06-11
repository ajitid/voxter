# Plan: Linux themed `voxter-symbolic` tray icon and installed assets

## Goal

Replace the Linux StatusNotifierItem pixmap tray icon with a KDE-theme-aware named icon:

- Expose `IconName = "voxter-symbolic"` from the `ksni` tray.
- Stop sending fixed black `IconPixmap` bytes on Linux.
- Ship a bundled symbolic SVG asset and install it into the system hicolor icon theme.
- Add a general Linux installer script at `scripts/install-linux.sh` with `--uninstall` and optional `--parts=...` selection.
- Remove the old `scripts/install-linux-helper.sh`; `scripts/install-linux.sh` owns all Linux install/uninstall flows.
- Since we now require an install script for Linux assets, stop embedding the audio files in the binary and install/read them from disk.

User decision recorded: use install/packaging, not runtime materialization. Add `scripts/install-linux.sh --uninstall`. Also undo embedded audio from commit `0d8a364ed8451d168d14fefe5f4d83afbd3d30d7` style and load installed audio files instead.

## Feasibility note

This is possible for the Voxter icon if the Linux icon is provided as a proper symbolic SVG, not as the current rendered ARGB pixmap.

Important caveat: KDE/Plasma can theme/recolor symbolic icons only when they are loaded through the icon-theme pipeline and authored as symbolic icons. So the SVG should use KDE/Breeze-style color handling, e.g. `class="ColorScheme-Text"` + `fill="currentColor"` / `stroke="currentColor"`, instead of hardcoded black. The existing raqote bitmap renderer cannot be recolored by KDE once sent as pixmap.

## References checked

- KDE `KStatusNotifierItem` docs: https://api.kde.org/kstatusnotifieritem.html
  - KDE recommends passing icon names rather than pixmaps where possible because this lets the tray load the appropriate size, replace the icon with a theme-specific icon, and avoid implementations that do not support pixmaps.
- StatusNotifierItem spec: https://specifications.freedesktop.org/status-notifier-item/latest-single/
  - Supports `IconName`, `IconPixmap`, and `IconThemePath`.
  - `IconPixmap` data is ARGB32; this is what we should avoid for Linux theming.
- KDE/Plasma 6 icon theming guidance: https://pointieststick.com/2023/08/12/how-all-this-icon-stuff-is-going-to-work-in-plasma-6/
  - Use `-symbolic` icon names when a symbolic/panel/tray icon is desired.
- Plasma `IconThemePath` behavior:
  - KDE MR says SNI can provide `IconThemePath` where tray visualization should look for icons: https://invent.kde.org/frameworks/plasma-framework/-/merge_requests/512
  - Plasma bug 479712 confirms `IconThemePath` support was fixed in Plasma 6.0.1, but using the installed hicolor theme avoids relying on a custom SNI path for the main flow: https://bugs.kde.org/show_bug.cgi?id=479712
- Local Breeze symbolic microphone SVG inspected at `/usr/share/icons/breeze/status/22/microphone-sensitivity-medium-symbolic.svg`; it uses `ColorScheme-Text` and `currentColor`.
- Current code:
  - Linux tray module: `src/ui/linux_tray.rs`
  - Shared pixmap art: `src/ui/tray_art.rs`
  - Embedded audio: `src/main.rs` `ON_SOUND` / `OFF_SOUND` via `include_bytes!`
  - Existing helper installer: `scripts/install-linux-helper.sh`
  - Linux docs: `docs/linux-wayland.md`

## Desired installed layout

Use fixed `/usr/local` paths, consistent with the existing helper installer:

```text
/usr/local/bin/voxter                                  # optional, but recommended for new all-in-one installer
/usr/local/libexec/voxter-hotkey-helper                # existing helper
/usr/local/share/voxter/assets/on.mp3
/usr/local/share/voxter/assets/off.mp3
/usr/share/icons/hicolor/scalable/status/voxter-symbolic.svg
/usr/share/polkit-1/actions/com.ajitid.voxter.hotkey-helper.policy
/usr/share/polkit-1/rules.d/50-voxter-hotkey-helper.rules
```

If installing the main `voxter` binary into `/usr/local/bin` feels too broad during implementation, keep it optional behind a script variable/flag. But the simplest `scripts/install-linux.sh` should build and install both binaries plus assets so a Linux run is self-contained.

## Implementation plan

### 1. Add symbolic SVG asset

Create:

```text
assets/icons/hicolor/scalable/status/voxter-symbolic.svg
```

SVG requirements:

- `viewBox="0 0 22 22"` or `0 0 24 24`; prefer 22 to match KDE small tray sizing.
- Use SVG path data approximating the existing side-on shock-mount icon from `src/ui/tray_art.rs`.
- Use KDE symbolic color pattern:

```xml
<style id="current-color-scheme" type="text/css">
  .ColorScheme-Text { color:#232629; }
</style>
<g class="ColorScheme-Text" fill="none" stroke="currentColor" ...>
```

- Avoid hardcoded black strokes/fills except as default `color` in the style block.
- Keep strokes simple and pixel-aligned enough at 22px.

Suggested SVG structure:

```xml
<svg viewBox="0 0 22 22" xmlns="http://www.w3.org/2000/svg">
  <style id="current-color-scheme" type="text/css">
    .ColorScheme-Text { color:#232629; }
  </style>
  <g class="ColorScheme-Text" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round">
    <path d="M4.6 10.8H17.4" stroke-width="1.6"/>
    <path d="M5.8 6.1 4.7 10.8 5.8 15.5M16.2 6.1 17.3 10.8 16.2 15.5" stroke-width="1.6"/>
    <g opacity=".6" stroke-width="1.15">
      <path d="M6.7 7 5.8 5.6M15.3 7 16.2 5.6M6.7 14.6 5.8 16M15.3 14.6 16.2 16"/>
      <path d="M6 6.3 11 10.8 16 6.3M6 15.3 11 10.8 16 15.3M7.8 6.2 14.2 15.4M14.2 6.2 7.8 15.4"/>
    </g>
  </g>
</svg>
```

During implementation, compare against the existing macOS tray icon and adjust if needed.

### 2. Update Linux tray to use icon name only

In `src/ui/linux_tray.rs`:

- Remove `Icon` field from `VoxterLinuxTray`.
- Remove `build_ksni_icon()`.
- Remove `icon_pixmap()` override, or make it return `Vec::new()`.
- Add:

```rust
fn icon_name(&self) -> String {
    "voxter-symbolic".to_string()
}
```

- Keep tooltip/menu behavior unchanged.

Expected struct:

```rust
struct VoxterLinuxTray {
    sender: AppSender,
    has_last_transcript: bool,
}
```

Expected `new()`:

```rust
let tray = VoxterLinuxTray { sender, has_last_transcript };
let handle = tray.spawn()...;
```

Do not use `IconPixmap` as a fallback, because that reintroduces the non-themed fixed-pixel problem.

### 3. Add strict installed-asset validation for Linux

Because Linux now depends on installed assets, add a startup check in Linux `run_app()` before creating the tray/audio manager.

Add constants near Linux-specific code in `src/main.rs`:

```rust
#[cfg(target_os = "linux")]
const LINUX_ON_SOUND_PATH: &str = "/usr/local/share/voxter/assets/on.mp3";
#[cfg(target_os = "linux")]
const LINUX_OFF_SOUND_PATH: &str = "/usr/local/share/voxter/assets/off.mp3";
#[cfg(target_os = "linux")]
const LINUX_TRAY_ICON_PATH: &str = "/usr/share/icons/hicolor/scalable/status/voxter-symbolic.svg";
```

Add:

```rust
#[cfg(target_os = "linux")]
fn validate_linux_installed_assets() -> Result<(), String> {
    for path in [LINUX_ON_SOUND_PATH, LINUX_OFF_SOUND_PATH, LINUX_TRAY_ICON_PATH] {
        if !std::path::Path::new(path).is_file() {
            return Err(format!(
                "Required Linux asset is missing: {path}. Install Voxter with: scripts/install-linux.sh"
            ));
        }
    }
    Ok(())
}
```

Call it in Linux `run_app()` after `WAYLAND_DISPLAY` check:

```rust
validate_linux_installed_assets()?;
```

This is intentionally strict and avoids silent placeholder icons or missing sounds.

### 4. Stop embedding audio; load platform paths from disk

In `src/main.rs`:

- Remove:

```rust
static ON_SOUND: &[u8] = include_bytes!("../assets/on.mp3");
static OFF_SOUND: &[u8] = include_bytes!("../assets/off.mp3");
```

- Restore a path-based `play_sound()` similar to pre-`0d8a364`:

```rust
fn play_sound<P: AsRef<std::path::Path>>(path: P) { ... File::open ... BufReader ... }
```

- Add platform sound path helpers:

```rust
#[cfg(target_os = "macos")]
const ON_SOUND_PATH: &str = "assets/on.mp3";
#[cfg(target_os = "macos")]
const OFF_SOUND_PATH: &str = "assets/off.mp3";

#[cfg(target_os = "linux")]
const ON_SOUND_PATH: &str = LINUX_ON_SOUND_PATH;
#[cfg(target_os = "linux")]
const OFF_SOUND_PATH: &str = LINUX_OFF_SOUND_PATH;
```

- Replace calls:

```rust
play_sound("on.mp3", ON_SOUND);
play_sound("off.mp3", OFF_SOUND);
```

with:

```rust
play_sound(ON_SOUND_PATH);
play_sound(OFF_SOUND_PATH);
```

Note: macOS direct `cargo run` still expects `assets/on.mp3` relative to repo/current working directory, matching the old behavior.

### 5. Add `scripts/install-linux.sh` and remove helper-only installer

Create a new all-in-one Linux installer:

```text
scripts/install-linux.sh
```

Remove the old helper-only installer from the repo:

```text
scripts/install-linux-helper.sh
```

Behavior:

```text
scripts/install-linux.sh [--install|--uninstall] [--parts=helper,app-binary,app-assets]
```

Parts:

- `helper`: `voxter-hotkey-helper` binary plus polkit policy/rules.
- `app-binary`: main `voxter` binary.
- `app-assets`: installed runtime assets: on/off mp3 files and `voxter-symbolic.svg` hicolor icon.

Default when `--parts` is omitted: install/uninstall all parts:

```text
helper,app-binary,app-assets
```

Examples:

```sh
scripts/install-linux.sh
scripts/install-linux.sh --uninstall
scripts/install-linux.sh --parts=app-assets
scripts/install-linux.sh --uninstall --parts=helper
```

Argument parsing requirements:

- Accept `--parts=value` only. No need to support `--parts value` unless desired.
- Split comma-separated values.
- Reject unknown parts with a clear error.
- Treat duplicate parts idempotently.
- `-h|--help` prints usage and part descriptions.

Install should:

1. `cd` to repo root.
2. Build only required binaries:
   - If `app-binary` selected: include `--bin voxter`.
   - If `helper` selected: include `--bin voxter-hotkey-helper`.
   - If both selected: build both in one cargo command.
   - If only `app-assets` selected: do not run cargo build.
3. Install selected binaries/assets:

```sh
# app-binary part
sudo install -Dm755 target/release/voxter /usr/local/bin/voxter

# helper part
sudo install -Dm755 target/release/voxter-hotkey-helper /usr/local/libexec/voxter-hotkey-helper
sudo install -Dm644 \
  packaging/polkit/com.ajitid.voxter.hotkey-helper.policy \
  /usr/share/polkit-1/actions/com.ajitid.voxter.hotkey-helper.policy
sudo install -Dm644 \
  packaging/polkit/50-voxter-hotkey-helper.rules \
  /usr/share/polkit-1/rules.d/50-voxter-hotkey-helper.rules

# app-assets part
sudo install -Dm644 assets/on.mp3 /usr/local/share/voxter/assets/on.mp3
sudo install -Dm644 assets/off.mp3 /usr/local/share/voxter/assets/off.mp3
sudo install -Dm644 assets/icons/hicolor/scalable/status/voxter-symbolic.svg \
  /usr/share/icons/hicolor/scalable/status/voxter-symbolic.svg
```

Important: preserve the existing policy path/name exactly: `com.ajitid.voxter.hotkey-helper.policy`.

After icon install, refresh caches if tools are present:

```sh
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  sudo gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi
if command -v kbuildsycoca6 >/dev/null 2>&1; then
  kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
elif command -v kbuildsycoca5 >/dev/null 2>&1; then
  kbuildsycoca5 --noincremental >/dev/null 2>&1 || true
fi
```

Cache refresh failures should print a warning, not hide a failed install of the actual files. If strictness is desired, make `gtk-update-icon-cache` failure fatal only when the command exists; decide during implementation based on local behavior.

Uninstall should remove only selected parts:

```text
# app-binary part
/usr/local/bin/voxter

# helper part
/usr/local/libexec/voxter-hotkey-helper
/usr/share/polkit-1/actions/com.ajitid.voxter.hotkey-helper.policy
/usr/share/polkit-1/rules.d/50-voxter-hotkey-helper.rules

# app-assets part
/usr/local/share/voxter/assets/on.mp3
/usr/local/share/voxter/assets/off.mp3
/usr/local/share/voxter              # rmdir if empty after asset removal
/usr/share/icons/hicolor/scalable/status/voxter-symbolic.svg
```

Then refresh caches similarly if `app-assets` was selected.

Update docs to use `scripts/install-linux.sh` as canonical. Do not mention `scripts/install-linux-helper.sh` except possibly in a migration note saying it was removed.

### 6. Update docs

In `docs/linux-wayland.md`:

- Replace helper-only install instructions:

```sh
scripts/install-linux-helper.sh
```

with:

```sh
scripts/install-linux.sh
```

- Add uninstall:

```sh
scripts/install-linux.sh --uninstall
```

- Update installed files list to include:

```text
/usr/local/bin/voxter
/usr/local/share/voxter/assets/on.mp3
/usr/local/share/voxter/assets/off.mp3
/usr/share/icons/hicolor/scalable/status/voxter-symbolic.svg
```

- Mention that Linux startup is strict and requires installed assets; if missing, rerun `scripts/install-linux.sh`.

### 7. Remove now-unused Linux pixmap art dependency if possible

After Linux tray stops using `src/ui/tray_art.rs`, it remains needed by macOS tray.

No dependency removal is expected because `raqote` is still used by macOS tray through `tray_art.rs`. However, if `tray_art.rs` is now macOS-only, decide whether to gate it:

```rust
#[cfg(target_os = "macos")]
pub mod tray_art;
```

But if keeping it unconditional is harmless, leave it to minimize churn.

### 8. Verification

Run:

```sh
cargo fmt
cargo check
cargo check --bin voxter-hotkey-helper
cargo clippy --all-targets
```

Installer dry-ish checks:

```sh
bash -n scripts/install-linux.sh
test ! -e scripts/install-linux-helper.sh
```

Manual Linux/KDE test:

1. Run `scripts/install-linux.sh`.
2. Confirm installed files exist:
   - `/usr/share/icons/hicolor/scalable/status/voxter-symbolic.svg`
   - `/usr/local/share/voxter/assets/on.mp3`
   - `/usr/local/share/voxter/assets/off.mp3`
3. Start `/usr/local/bin/voxter` on KDE Wayland.
4. Confirm tray icon appears as Voxter custom artwork.
5. Switch Plasma light/dark or Breeze/Breeze Dark and confirm the tray icon recolors with the panel/theme.
6. Confirm no fixed black pixmap remains in the Linux tray.
7. Confirm on/off sounds still play from installed files.
8. Run `scripts/install-linux.sh --uninstall`.
9. Confirm starting Linux Voxter fails loudly due to missing installed assets.

## Non-goals

- Do not use a generic `audio-input-microphone-symbolic` icon for the final option-2 implementation.
- Do not reintroduce Linux tray `IconPixmap` fallback.
- Do not runtime-materialize the icon into `XDG_RUNTIME_DIR`; user chose install/packaging instead.
- Do not add a Linux keyboard shortcut for retyping.
