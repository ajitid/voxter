# Plan: GNOME Shell extension overlay for Linux Wayland

## Goal

Replace the Linux/GNOME Wayland winit overlay window with a GNOME Shell extension-rendered overlay so the recording UI does not steal focus and does not fall behind normal windows.

macOS should keep the current winit/wgpu overlay.

## User decisions

- On GNOME Wayland, do **not** keep the winit overlay implementation as a fallback.
- If the GNOME Shell extension is missing/disabled/unreachable, treat it as a startup error on GNOME Wayland.
- Preserve full-ish current visual fidelity:
  - app sends overlay state plus live speech level over D-Bus;
  - GNOME Shell extension draws/animates the bottom-center overlay from that data.

## Research findings

### Why the current Linux overlay cannot be fixed with winit

Current code creates a normal winit toplevel overlay in `src/ui/overlay.rs`:

```rust
.with_decorations(false)
.with_window_level(WindowLevel::AlwaysOnTop)
...
self.window.set_visible(true);
```

Relevant winit 0.30.13 source findings:

- `WindowLevel` docs in `/home/ajit/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/winit-0.30.13/src/window.rs` say Wayland is unsupported.
- Wayland implementation in `/home/ajit/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/winit-0.30.13/src/platform_impl/linux/wayland/window/mod.rs` has:
  - `pub fn set_window_level(&self, _level: WindowLevel) {}`
  - `pub fn set_outer_position(&self, _: Position) { // Not possible on Wayland. }`
- `WindowAttributes::with_active()` docs say Wayland/X11 are unsupported.
- `set_cursor_hittest(false)` does work on Wayland by setting an empty input region, but that only passes pointer input through; it does not prevent focus assignment when the window is mapped.

Web references:

- Electron Wayland overview: apps cannot unilaterally move, resize, or focus windows on Wayland; compositor/portals/protocols mediate this. <https://www.electronjs.org/blog/tech-talk-wayland>
- GNOME Shell focus stealing prevention: focus is intentionally compositor-mediated for security. <https://blogs.gnome.org/shell-dev/2024/09/20/understanding-gnome-shells-focus-stealing-prevention/>
- Layer-shell supports non-keyboard-interactive overlay surfaces, but GNOME/Mutter does not support `zwlr_layer_shell_v1`; so it is not a viable GNOME solution. Example discussion: <https://unix.stackexchange.com/questions/800865/rofi-2-0-crashes-on-debian-13-gnome-shell-48-wayland-due-to-missing-layer-shel>

Conclusion: on GNOME Wayland, a true non-focus-stealing, above-window overlay must be drawn inside GNOME Shell itself, i.e. via a Shell extension.

### GNOME Shell extension and D-Bus implementation references

- GNOME extension creation guide: <https://gjs.guide/extensions/development/creating.html>
- GNOME 45+ extension import/class format: <https://blogs.gnome.org/shell-dev/author/fmuellner/> and GNOME 45 porting guide references there.
- GJS D-Bus guide: `Gio.DBusExportedObject.wrapJSObject()` can export JS objects as D-Bus services. <https://gjs.guide/guides/gio/dbus.html>
- Example of sending strings to a Shell extension over D-Bus: <https://stackoverflow.com/questions/33001192/how-to-send-a-string-to-a-gnome-shell-extension>

Current machine reports `GNOME Shell 50.2`, so the extension should use the GNOME 45+ ES module style and include shell-version entries through 50.

## Target architecture

### D-Bus service exported by GNOME Shell extension

GNOME Shell extension owns session bus name:

```text
com.ajitid.VoxtralSpeechToText.Overlay
```

Object path:

```text
/com/ajitid/VoxtralSpeechToText/Overlay
```

Interface:

```text
com.ajitid.VoxtralSpeechToText.Overlay1
```

Interface XML:

```xml
<node>
  <interface name="com.ajitid.VoxtralSpeechToText.Overlay1">
    <method name="Ping">
      <arg type="s" name="version" direction="out"/>
    </method>
    <method name="SetOverlay">
      <arg type="s" name="state" direction="in"/>
      <arg type="d" name="level" direction="in"/>
    </method>
  </interface>
</node>
```

Valid `state` strings:

- `hidden`
- `recording`
- `recording_latch`
- `transcribing`

`level` is a `double` clamped by both app and extension to `0.0..=1.0`.

### App-side behavior

- macOS: keep current winit/wgpu overlay.
- Linux GNOME Wayland:
  - do not create a winit overlay window;
  - connect to the extension D-Bus service;
  - startup fails if `Ping()` cannot be called;
  - send `SetOverlay(state, level)` on state changes;
  - while visible, send throttled level updates at about 30 Hz from existing `SpeechVizState::level()`;
  - hide/reset via `SetOverlay("hidden", 0.0)`.

### Extension-side behavior

- Adds a non-reactive/non-focusable St actor to GNOME Shell chrome/UI group.
- Draws a 420x96 transparent bottom-center overlay on the primary monitor.
- Does not participate in input/focus (`reactive: false`, `can_focus: false`, and add chrome with non-input-affecting params where available).
- Repositions on monitor changes.
- Recording states draw an arc whose sagitta follows `level.pow(0.72)` like current Rust renderer.
- Latch state adds a simple lock glyph/icon near the arc end.
- Transcribing draws animated colored squares/spinner locally on a GLib timeout.
- Hidden state hides the actor and stops animation timers.

## Implementation plan

### 1. Add GNOME Shell extension files

Create directory:

```text
gnome-shell-extension/voxtral-speech-to-text@ajitid/
```

Add `metadata.json`:

```json
{
  "uuid": "voxtral-speech-to-text@ajitid",
  "name": "Voxtral Speech-to-Text Overlay",
  "description": "GNOME Shell overlay for Voxtral Speech-to-Text recording status",
  "shell-version": ["45", "46", "47", "48", "49", "50"],
  "version": 1
}
```

Add `stylesheet.css` with classes:

```css
.voxtral-overlay-container {
  width: 420px;
  height: 96px;
}
```

Add `extension.js` using GNOME 45+ module style:

```js
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
```

Implementation details:

- export default class extends `Extension`.
- In `enable()`:
  - create `St.DrawingArea` or `St.Widget`/`Clutter.Canvas` sized 420x96;
  - set `reactive: false`, `can_focus: false`, `visible: false`, style class `voxtral-overlay-container`;
  - add to Shell chrome, preferably:
    ```js
    Main.layoutManager.addTopChrome(this._actor, {
      affectsInputRegion: false,
      affectsStruts: false,
      trackFullscreen: true,
    });
    ```
    If the exact GNOME 50 API rejects one option, remove only the unsupported option during implementation after testing against Looking Glass/logs.
  - connect `monitors-changed` to reposition;
  - export D-Bus object with `Gio.DBusExportedObject.wrapJSObject()`;
  - own bus name with `Gio.DBus.session.own_name()`.
- In `disable()`:
  - hide overlay;
  - remove GLib timeout source;
  - unexport D-Bus object;
  - unown bus name;
  - disconnect signals;
  - remove/destroy actor.
- D-Bus methods:
  - `Ping()` returns a small version string, e.g. `"1"`.
  - `SetOverlay(state, level)` validates state, clamps level, stores fields, shows/hides actor, queues repaint.
- Drawing:
  - For recording/latch, port the current arc math from `src/ui/render.rs::draw_frame()`:
    - width 420, height 96;
    - `display_energy = level.pow(0.72)`;
    - `cx = width * 0.5`;
    - `half_chord = min(width * 0.26, width * 0.42).max(max(width * 0.18, 48.0))`;
    - `y_base = height - 14.0`;
    - `sagitta = 1.8 + display_energy * (height * 0.42)`;
    - sample ~56 points.
  - Cairo does not need to perfectly reproduce the current wgpu gradient on day one; use a close multi-stop linear gradient if available, otherwise use a solid accent color.
  - For transcribing, use a GLib timeout around 30/60 Hz and draw 3 animated squares using elapsed monotonic time.

### 2. Add install helper script

Create:

```text
scripts/install-gnome-shell-extension.sh
```

Behavior:

```bash
#!/usr/bin/env bash
set -euo pipefail
uuid="voxtral-speech-to-text@ajitid"
src_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/gnome-shell-extension/$uuid"
dst_dir="$HOME/.local/share/gnome-shell/extensions/$uuid"
rm -rf "$dst_dir"
mkdir -p "$(dirname "$dst_dir")"
cp -a "$src_dir" "$dst_dir"
gnome-extensions enable "$uuid" || true
cat <<'MSG'
Installed Voxtral GNOME Shell extension.
On Wayland, log out and log back in if GNOME Shell has not loaded the new extension yet.
Verify with: gnome-extensions info voxtral-speech-to-text@ajitid
MSG
```

Use `chmod +x` during implementation.

### 3. Split overlay implementation by platform

Current file `src/ui/overlay.rs` is winit/wgpu-based and used on both macOS and Linux.

Refactor to platform modules:

```text
src/ui/overlay.rs                # public OverlayState + re-export platform controller
src/ui/overlay/macos.rs          # current winit/wgpu implementation
src/ui/overlay/linux.rs          # D-Bus GNOME Shell extension implementation
```

Because Rust cannot have both `src/ui/overlay.rs` and `src/ui/overlay/` without declaring submodules carefully, use this layout:

- Keep `src/ui/overlay.rs` as the public module.
- Move current implementation code into `src/ui/overlay/macos.rs`.
- Add `src/ui/overlay/linux.rs`.
- In `src/ui/overlay.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayState {
    Hidden,
    Recording,
    RecordingLatch,
    Transcribing,
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::OverlayController;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::OverlayController;
```

### 4. macOS overlay module changes

In `src/ui/overlay/macos.rs`, use the current code from `src/ui/overlay.rs` mostly unchanged.

Important API change: make `window_id()` return `WindowId` only in the macOS controller. Linux will not have a window id.

### 5. Linux overlay controller over D-Bus

Create `src/ui/overlay/linux.rs`.

Use `ashpd::zbus` re-export already available from the Linux `ashpd` dependency; no new dependency should be necessary.

Sketch:

```rust
use crate::SpeechVizState;
use crate::ui::overlay::OverlayState;
use ashpd::zbus;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::event_loop::ActiveEventLoop;

const BUS_NAME: &str = "com.ajitid.VoxtralSpeechToText.Overlay";
const OBJECT_PATH: &str = "/com/ajitid/VoxtralSpeechToText/Overlay";
const INTERFACE: &str = "com.ajitid.VoxtralSpeechToText.Overlay1";
const MIN_SEND_INTERVAL: Duration = Duration::from_millis(33);

pub struct OverlayController {
    speech_viz: Arc<SpeechVizState>,
    connection: zbus::blocking::Connection,
    state: OverlayState,
    last_sent_at: Option<Instant>,
    last_sent_level: f32,
}
```

Need verify exact zbus 5 blocking API during implementation. If `ashpd` re-export does not expose blocking API with current enabled features, add direct Linux dependency:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
zbus = { version = "5", default-features = false, features = ["blocking", "async-io"] }
```

Preferred implementation:

- `OverlayController::new(_: &ActiveEventLoop, speech_viz: Arc<SpeechVizState>) -> Result<Self, String>`:
  - optionally verify GNOME Wayland environment:
    - `XDG_SESSION_TYPE=wayland`
    - `XDG_CURRENT_DESKTOP` contains `GNOME`
  - create session bus connection;
  - call `Ping()` on the extension;
  - if it fails, return a clear error:
    ```text
    GNOME Shell overlay extension is required on GNOME Wayland but is not available. Install with scripts/install-gnome-shell-extension.sh, enable it, then log out/in.
    ```
- `is_visible()` returns `state != Hidden`.
- `update_state(_, next_state)` stores state and sends immediately.
- `request_redraw()` sends state+current level if visible and at least 33 ms elapsed or level changed materially.
- No winit window is created.
- `redraw()` can be omitted for Linux if main is cfg-split; otherwise make it a no-op returning `Ok(())`.

State conversion:

```rust
fn state_name(state: OverlayState) -> &'static str {
    match state {
        OverlayState::Hidden => "hidden",
        OverlayState::Recording => "recording",
        OverlayState::RecordingLatch => "recording_latch",
        OverlayState::Transcribing => "transcribing",
    }
}
```

D-Bus call helper:

```rust
fn send_overlay(&mut self, force: bool) -> Result<(), String> {
    let now = Instant::now();
    if !force
        && self.last_sent_at.is_some_and(|last| now.duration_since(last) < MIN_SEND_INTERVAL)
    {
        return Ok(());
    }

    let level = if self.speech_viz.is_active() { self.speech_viz.level() } else { 0.0 }
        .clamp(0.0, 1.0);

    let proxy = zbus::blocking::Proxy::new(
        &self.connection,
        BUS_NAME,
        OBJECT_PATH,
        INTERFACE,
    )
    .map_err(|e| format!("GNOME Shell overlay D-Bus proxy error: {e}"))?;

    proxy
        .call_method("SetOverlay", &(state_name(self.state), level as f64))
        .map_err(|e| format!("GNOME Shell overlay update failed: {e}"))?;

    self.last_sent_at = Some(now);
    self.last_sent_level = level;
    Ok(())
}
```

During implementation, adjust for exact zbus return types.

### 6. Update `src/main.rs` for platform overlay behavior

Current imports:

```rust
use winit::event::WindowEvent;
use winit::window::WindowId;
```

Change to cfg-gate window-specific types where possible:

```rust
#[cfg(target_os = "macos")]
use winit::event::WindowEvent;
#[cfg(target_os = "macos")]
use winit::window::WindowId;
```

Change `App` fields:

```rust
struct App {
    audio_manager: AudioManager,
    overlay: Option<OverlayController>,
    #[cfg(target_os = "macos")]
    overlay_window_id: Option<WindowId>,
    tray: Option<StatusTray>,
    proxy: EventLoopProxy<AppEvent>,
}
```

Adjust `App::new()` accordingly.

In `resumed()`:

- On macOS, keep current behavior and store `overlay.window_id()`.
- On Linux, if `OverlayController::new(...)` fails, print error and `event_loop.exit()` immediately. Do not continue to hotkey operation without overlay.

Patch shape:

```rust
if self.overlay.is_none() {
    match OverlayController::new(event_loop, Arc::clone(&self.audio_manager.speech_viz)) {
        Ok(overlay) => {
            #[cfg(target_os = "macos")]
            {
                self.overlay_window_id = Some(overlay.window_id());
            }
            self.overlay = Some(overlay);
        }
        Err(e) => {
            eprintln!("Overlay initialization failed: {e}");
            #[cfg(target_os = "linux")]
            event_loop.exit();
        }
    }
}
```

Change `window_event()`:

- macOS only handles winit overlay window events.
- Linux should not have a `window_event` body dependent on overlay window APIs.

Implementation options:

```rust
#[cfg(target_os = "macos")]
fn window_event(... WindowEvent ...) { current body }
```

If trait requires method on Linux too, keep method with fully-qualified type and immediately ignore args, but do not call overlay `window()`/`handle_resize()`/`redraw()` on Linux.

In `about_to_wait()`, keep:

```rust
if let Some(overlay) = self.overlay.as_ref()
    && overlay.is_visible()
{
    overlay.request_redraw();
}
```

For Linux this becomes the throttled D-Bus level update loop.

### 7. Startup check placement

Because user chose hard requirement, prefer failing before registering hotkeys if possible.

Option A (recommended): after creating the winit event loop but before `spawn_hotkey_listener()`, call a Linux-only preflight:

```rust
#[cfg(target_os = "linux")]
check_gnome_shell_overlay_extension_available()?;
```

This preflight performs the same D-Bus `Ping()`. Then `OverlayController::new()` can still verify again.

If using blocking zbus in the overlay module, expose:

```rust
#[cfg(target_os = "linux")]
pub fn check_overlay_available() -> Result<(), String>
```

and call it from `main()`.

This gives an immediate terminal error instead of exiting only after `resumed()`.

### 8. Remove Linux winit overlay path

After platform split:

- `src/ui/render.rs` remains compiled for macOS only if practical:
  - add `#[cfg(target_os = "macos")] pub mod render;` in `src/ui/mod.rs`, or
  - leave it compiled if easier; it is not used by Linux overlay.
- Ensure Linux `src/ui/overlay/linux.rs` does not import `OverlayRenderer`, `Window`, `WindowLevel`, `wgpu`, or create a window.
- Keep `winit` dependency because the application event loop still uses it.

### 9. Update documentation

Add `docs/linux-gnome-shell-overlay.md` with:

- why GNOME Wayland needs a Shell extension for the overlay;
- install command:
  ```bash
  scripts/install-gnome-shell-extension.sh
  ```
- enable/check commands:
  ```bash
  gnome-extensions enable voxtral-speech-to-text@ajitid
  gnome-extensions info voxtral-speech-to-text@ajitid
  busctl --user call com.ajitid.VoxtralSpeechToText.Overlay /com/ajitid/VoxtralSpeechToText/Overlay com.ajitid.VoxtralSpeechToText.Overlay1 Ping
  ```
- note: on Wayland, log out/in after first install if GNOME Shell has not loaded the extension.
- note: the app intentionally fails startup on GNOME Wayland if the extension is unavailable.

Update `docs/linux-workarounds.md`:

- replace/augment the current direct winit overlay explanation with a short link to `docs/linux-gnome-shell-overlay.md`.
- mention that Linux typing still uses RemoteDesktop, independent of overlay rendering.

Update `CLAUDE.md` Linux environment notes:

- mention GNOME Shell extension is required for visual overlay on GNOME Wayland.

### 10. Add todo file during implementation

Create `.pi/todos/linux-gnome-shell-overlay-extension.md` with phases:

- Extension files.
- Install script.
- Rust platform split.
- Linux D-Bus overlay controller.
- Startup hard requirement.
- Docs.
- Verification.

## Verification plan

### Static/build checks

Run:

```bash
cargo fmt
cargo check
cargo clippy
```

If possible, also run macOS build/check on macOS later to verify the moved winit overlay still compiles there.

### Extension install/check

```bash
scripts/install-gnome-shell-extension.sh
gnome-extensions info voxtral-speech-to-text@ajitid
busctl --user call \
  com.ajitid.VoxtralSpeechToText.Overlay \
  /com/ajitid/VoxtralSpeechToText/Overlay \
  com.ajitid.VoxtralSpeechToText.Overlay1 \
  Ping
```

Expected: returns version string.

Manual state checks:

```bash
busctl --user call \
  com.ajitid.VoxtralSpeechToText.Overlay \
  /com/ajitid/VoxtralSpeechToText/Overlay \
  com.ajitid.VoxtralSpeechToText.Overlay1 \
  SetOverlay sd recording 0.6

busctl --user call \
  com.ajitid.VoxtralSpeechToText.Overlay \
  /com/ajitid/VoxtralSpeechToText/Overlay \
  com.ajitid.VoxtralSpeechToText.Overlay1 \
  SetOverlay sd recording_latch 0.8

busctl --user call \
  com.ajitid.VoxtralSpeechToText.Overlay \
  /com/ajitid/VoxtralSpeechToText/Overlay \
  com.ajitid.VoxtralSpeechToText.Overlay1 \
  SetOverlay sd transcribing 0.0

busctl --user call \
  com.ajitid.VoxtralSpeechToText.Overlay \
  /com/ajitid/VoxtralSpeechToText/Overlay \
  com.ajitid.VoxtralSpeechToText.Overlay1 \
  SetOverlay sd hidden 0.0
```

Expected:

- overlay appears bottom-center above windows;
- does not take focus;
- clicking another app while overlay is visible focuses that app normally;
- overlay remains above normal app windows;
- hidden removes it.

### App behavior checks

1. Disable extension:
   ```bash
   gnome-extensions disable voxtral-speech-to-text@ajitid
   cargo run
   ```
   Expected: startup fails with clear extension-required error on GNOME Wayland.
2. Enable extension and restart/log out-in if needed:
   ```bash
   gnome-extensions enable voxtral-speech-to-text@ajitid
   cargo run
   ```
   Expected: app starts.
3. Trigger recording:
   - overlay appears without focus stealing;
   - speech arc responds to voice level;
   - latch lock appears in Linux latch mode;
   - transcribing animation appears;
   - overlay hides after transcription/no-speech/error.
4. With another window focused, trigger record/transcribe and confirm typed output still goes to intended focused app; overlay never becomes focused.

## Risks / notes

- GNOME Shell extension APIs are not stable across major versions. Current plan targets GNOME 45+ ES modules and includes shell versions 45-50. Future GNOME updates may require extension maintenance.
- `Main.layoutManager.addTopChrome()` option names should be validated on GNOME 50. If `affectsInputRegion` is unsupported, remove just that option and rely on `reactive:false`/`can_focus:false` plus Shell chrome behavior.
- If D-Bus calls at 30 Hz are too expensive, throttle to 15-20 Hz or only send when level delta exceeds a threshold. Start at 33 ms because current overlay already polls while visible.
- Shell extension drawing will approximate current wgpu/raqote visuals; exact font/gradient parity is not required for first implementation, but state semantics and no-focus behavior are required.
