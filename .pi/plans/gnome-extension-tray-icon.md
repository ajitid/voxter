# Plan: GNOME Shell panel tray icon without AppIndicator/GTK

## Goal

Add Linux/GNOME tray/menu support equivalent to macOS menu-bar support without AppIndicator on Linux. `tray-icon`/`muda` are acceptable where they do not require AppIndicator, but current `tray-icon`'s Linux tray backend is AppIndicator-based, so the Linux implementation should use the GNOME Shell extension instead.

Menu options to match macOS:

1. `Type last transcript` — disabled until a transcript exists.
2. separator
3. `Quit`

Use the existing bundled GNOME Shell extension as the Linux panel/tray implementation.

## Current repo facts

- The project already ships a GNOME Shell extension:
  - `gnome-shell-extension/voxtral-speech-to-text@ajitid/extension.js`
  - `gnome-shell-extension/voxtral-speech-to-text@ajitid/metadata.json`
  - `gnome-shell-extension/voxtral-speech-to-text@ajitid/stylesheet.css`
- The extension currently owns D-Bus name:
  - `com.ajitid.VoxtralSpeechToText.Overlay`
  - object path `/com/ajitid/VoxtralSpeechToText/Overlay`
  - interface `com.ajitid.VoxtralSpeechToText.Overlay1`
- The Rust app already calls extension methods from:
  - `src/ui/overlay/linux.rs`
- Linux `tray-icon`, GTK, GTK4, and AppIndicator dependencies were removed. Current Linux build has no such deps.
- The app already has Linux D-Bus dependency:
  - `zbus = { version = "5", default-features = false, features = ["async-io", "blocking-api"] }`
- macOS tray code remains in:
  - `src/ui/tray.rs`
  - gated by `#[cfg(target_os = "macos")]`

## Important design clarification

AppIndicator is a Linux StatusNotifier/AppIndicator mechanism. It is not a macOS mechanism. macOS tray/menu-bar support uses native macOS status item APIs via `tray-icon`.

For GNOME Shell, a shell extension can directly add a top-panel button with a popup menu using GNOME Shell APIs (`PanelMenu`, `PopupMenu`, `St`). This is independent of AppIndicator and does not require GTK/AppIndicator libraries in the Rust app.

## Recommended architecture

Use the GNOME Shell extension for Linux panel UI. Keep `tray-icon` for macOS native menu-bar support. Do not use `tray-icon` as the Linux tray backend unless upstream gains a non-AppIndicator GNOME/native backend.

Use two D-Bus roles:

### 1. Extension-owned service: overlay + tray state

Keep the existing extension-owned bus name and interface:

- bus name: `com.ajitid.VoxtralSpeechToText.Overlay`
- object path: `/com/ajitid/VoxtralSpeechToText/Overlay`
- interface: `com.ajitid.VoxtralSpeechToText.Overlay1`

Add a method:

```xml
<method name="SetTrayState">
  <arg type="b" name="hasLastTranscript" direction="in"/>
</method>
```

Rust calls this when:

- app starts / extension connects
- transcript updates
- optionally when quitting: set `hasLastTranscript=false` or let name-watch handle app disappearance

Extension uses this to enable/disable `Type last transcript`.

### 2. App-owned service: menu actions

Add a Rust-owned D-Bus service while app is running:

- bus name: `com.ajitid.VoxtralSpeechToText.App`
- object path: `/com/ajitid/VoxtralSpeechToText/App`
- interface: `com.ajitid.VoxtralSpeechToText.App1`

Methods:

```xml
<method name="TypeLastTranscript"/>
<method name="Quit"/>
<method name="Ping">
  <arg type="s" name="version" direction="out"/>
</method>
```

Implementation behavior:

- `TypeLastTranscript` sends a new internal event to the winit event loop, e.g. `AppEvent::TypeLastTranscript`.
- `Quit` sends `AppEvent::Control(ControlMsg::Quit)`.
- `Ping` returns `"1"`.

Extension menu item activation calls these app-owned methods via D-Bus.

Why this direction:

- Keeps all real app behavior in Rust.
- Extension remains UI-only.
- No shell extension process spawning commands.
- Quit can reuse existing app shutdown logic.
- Type-last can reuse existing `type_last_transcript()`.
- The extension can watch the app bus name and disable menu items when the app is not running.

## Patch spec

### A. `gnome-shell-extension/.../extension.js`

#### Imports

Add imports:

```js
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
```

Optionally import GObject if subclassing is preferred. Simpler implementation can instantiate `new PanelMenu.Button(...)` directly without a class.

#### Constants

Add:

```js
const APP_BUS_NAME = 'com.ajitid.VoxtralSpeechToText.App';
const APP_OBJECT_PATH = '/com/ajitid/VoxtralSpeechToText/App';
const APP_INTERFACE = 'com.ajitid.VoxtralSpeechToText.App1';
```

#### D-Bus XML

Extend `IFACE_XML` with:

```xml
<method name="SetTrayState">
  <arg type="b" name="hasLastTranscript" direction="in"/>
</method>
```

#### `enable()` additions

Initialize state:

```js
this._appAvailable = false;
this._hasLastTranscript = false;
```

Create panel indicator:

```js
this._indicator = new PanelMenu.Button(0.0, 'Voxtral Speech-to-Text', false);
this._indicatorIcon = new St.Icon({
    icon_name: 'audio-input-microphone-symbolic',
    style_class: 'system-status-icon',
});
this._indicator.add_child(this._indicatorIcon);

this._typeItem = new PopupMenu.PopupMenuItem('Type last transcript');
this._typeItem.connect('activate', () => this._callApp('TypeLastTranscript'));
this._indicator.menu.addMenuItem(this._typeItem);

this._indicator.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

this._quitItem = new PopupMenu.PopupMenuItem('Quit');
this._quitItem.connect('activate', () => this._callApp('Quit'));
this._indicator.menu.addMenuItem(this._quitItem);

Main.panel.addToStatusArea('voxtral-speech-to-text', this._indicator, 0, 'right');
this._updateTrayMenu();
```

Watch app bus name:

```js
this._appWatchId = Gio.bus_watch_name(
    Gio.BusType.SESSION,
    APP_BUS_NAME,
    Gio.BusNameWatcherFlags.NONE,
    () => {
        this._appAvailable = true;
        this._updateTrayMenu();
    },
    () => {
        this._appAvailable = false;
        this._hasLastTranscript = false;
        this._updateTrayMenu();
    }
);
```

#### Add methods

```js
SetTrayState(hasLastTranscript) {
    this._hasLastTranscript = Boolean(hasLastTranscript);
    this._updateTrayMenu();
}

_updateTrayMenu() {
    if (this._typeItem)
        this._typeItem.setSensitive(this._appAvailable && this._hasLastTranscript);
    if (this._quitItem)
        this._quitItem.setSensitive(this._appAvailable);
}

_callApp(method) {
    if (!this._appAvailable)
        return;

    Gio.DBus.session.call(
        APP_BUS_NAME,
        APP_OBJECT_PATH,
        APP_INTERFACE,
        method,
        null,
        null,
        Gio.DBusCallFlags.NONE,
        -1,
        null,
        (_conn, res) => {
            try {
                Gio.DBus.session.call_finish(res);
            } catch (e) {
                logError(e, `Voxtral app D-Bus call failed: ${method}`);
            }
        }
    );
}
```

#### `disable()` additions

Before destroying overlay actor, clean up panel indicator and name watch:

```js
if (this._appWatchId) {
    Gio.bus_unwatch_name(this._appWatchId);
    this._appWatchId = 0;
}

if (this._indicator) {
    this._indicator.destroy();
    this._indicator = null;
}
this._typeItem = null;
this._quitItem = null;
this._indicatorIcon = null;
```

#### Metadata

Update description/name if desired:

- name: `Voxtral Speech-to-Text`
- description: `GNOME Shell overlay and panel menu for Voxtral Speech-to-Text`

### B. Rust: add Linux D-Bus app action service

Create new file:

- `src/ui/linux_app_dbus.rs` or `src/platform/linux_app_dbus.rs`

Recommended content shape:

```rust
#[cfg(target_os = "linux")]
pub struct LinuxAppDbusHandle {
    stop_tx: std::sync::mpsc::Sender<()>,
    join: Option<std::thread::JoinHandle<()>>,
}
```

Spawn function:

```rust
#[cfg(target_os = "linux")]
pub fn spawn_linux_app_dbus(proxy: winit::event_loop::EventLoopProxy<crate::AppEvent>) -> Result<LinuxAppDbusHandle, String>
```

Important: `AppEvent` is currently private. Either:

1. put the D-Bus action service in `src/main.rs`, or
2. make only the needed event type accessible with `pub(crate)`, or
3. pass a `std::sync::mpsc::Sender<LinuxTrayAction>` and bridge to `AppEvent` in `main.rs`.

Recommended minimal edit: keep the D-Bus service code in `src/main.rs` under `#[cfg(target_os = "linux")]` to avoid visibility churn.

Action object methods:

```rust
struct LinuxAppActions {
    proxy: EventLoopProxy<AppEvent>,
}

#[zbus::interface(name = "com.ajitid.VoxtralSpeechToText.App1")]
impl LinuxAppActions {
    fn ping(&self) -> &str { "1" }

    fn type_last_transcript(&self) {
        let _ = self.proxy.send_event(AppEvent::TypeLastTranscript);
    }

    fn quit(&self) {
        let _ = self.proxy.send_event(AppEvent::Control(ControlMsg::Quit));
    }
}
```

Thread loop outline:

```rust
fn spawn_linux_app_dbus(proxy: EventLoopProxy<AppEvent>) -> Result<LinuxAppDbusHandle, String> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel();

    let join = std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let connection = zbus::blocking::Connection::session()
                .map_err(|e| format!("Linux app D-Bus session connection failed: {e}"))?;
            connection
                .request_name("com.ajitid.VoxtralSpeechToText.App")
                .map_err(|e| format!("Failed to own app D-Bus name: {e}"))?;
            connection
                .object_server()
                .at("/com/ajitid/VoxtralSpeechToText/App", LinuxAppActions { proxy })
                .map_err(|e| format!("Failed to export app D-Bus object: {e}"))?;

            let _ = ready_tx.send(Ok(()));

            while stop_rx.try_recv().is_err() {
                connection
                    .object_server()
                    .try_handle_next(std::time::Duration::from_millis(100))
                    .map_err(|e| format!("App D-Bus server error: {e}"))?;
            }
            Ok(())
        })();

        if let Err(e) = result {
            let _ = ready_tx.send(Err(e));
        }
    });

    ready_rx.recv().map_err(|e| format!("App D-Bus startup channel failed: {e}"))??;
    Ok(LinuxAppDbusHandle { stop_tx, join: Some(join) })
}
```

`Drop` impl should send stop and join.

### C. Rust: add Linux tray action event

In `src/main.rs`, extend `AppEvent`:

```rust
#[cfg(target_os = "linux")]
TypeLastTranscript,
```

In `ApplicationHandler::user_event`:

```rust
#[cfg(target_os = "linux")]
AppEvent::TypeLastTranscript => match type_last_transcript() {
    Ok(true) => println!("Typed last transcript"),
    Ok(false) => println!("No last transcript available to type"),
    Err(e) => eprintln!("Failed to type transcript: {e}"),
},
```

Important: currently `last_transcription_text()` and `type_last_transcript()` were gated macOS-only after removing Linux tray. Remove that gate or change to:

```rust
#[cfg(any(target_os = "macos", target_os = "linux"))]
```

### D. Rust: hold Linux D-Bus handle in `App`

Add field:

```rust
#[cfg(target_os = "linux")]
_app_dbus: LinuxAppDbusHandle,
```

But this requires constructing it before `App::new`. Easier:

- start app D-Bus service in `main()` after `proxy` is created
- keep handle in local variable until `run_app` returns:

```rust
#[cfg(target_os = "linux")]
let _linux_app_dbus = spawn_linux_app_dbus(proxy.clone())?;
```

This avoids changing `App` struct.

### E. Rust: notify extension menu state

Extend `src/ui/overlay/linux.rs` with a method:

```rust
pub fn set_tray_state(&self, has_last_transcript: bool) -> Result<(), String> {
    let proxy = overlay_proxy(&self.connection)?;
    proxy
        .call::<_, _, ()>("SetTrayState", &(has_last_transcript,))
        .map_err(|e| format!("GNOME Shell tray state update failed: {e}"))
}
```

Then in `src/main.rs`, update on transcript changes:

```rust
AppEvent::TranscriptUpdated => {
    #[cfg(target_os = "macos")]
    self.refresh_tray_menu_state();
    #[cfg(target_os = "linux")]
    if let Some(overlay) = self.overlay.as_ref() {
        if let Err(e) = overlay.set_tray_state(last_transcription_text().is_some()) {
            eprintln!("Tray state update failed: {e}");
        }
    }
}
```

Also after overlay creation in `resumed()` on Linux:

```rust
#[cfg(target_os = "linux")]
if let Some(overlay) = self.overlay.as_ref() {
    let _ = overlay.set_tray_state(last_transcription_text().is_some());
}
```

This keeps `Type last transcript` enabled when appropriate.

### F. Install/reload script

`install-gnome-shell-extension.sh` already installs and enables the extension.

Update message to mention panel menu. Optionally improve reload story later, but not necessary for this feature.

## Verification plan

### Build/dependency verification

Run:

```sh
cargo fmt
cargo check
cargo tree --target x86_64-unknown-linux-gnu | rg 'gtk|gtk4|appindicator|tray-icon|muda' || true
rg -n 'gtk4|gtk =|libappindicator|appindicator' Cargo.toml Cargo.lock src gnome-shell-extension
```

Expected:

- `cargo check` passes.
- No GTK/GTK4/AppIndicator dependency in Linux tree.
- Only textual mentions of AppIndicator should be absent or in comments/docs if intentionally retained.

### Extension verification

Install:

```sh
scripts/install-gnome-shell-extension.sh
```

Then log out/in if needed.

Verify extension is loaded:

```sh
gnome-extensions info voxtral-speech-to-text@ajitid
```

Verify extension-owned D-Bus method exists:

```sh
gdbus call \
  --session \
  --dest com.ajitid.VoxtralSpeechToText.Overlay \
  --object-path /com/ajitid/VoxtralSpeechToText/Overlay \
  --method com.ajitid.VoxtralSpeechToText.Overlay1.Ping
```

Run app:

```sh
cargo run
```

Verify app-owned D-Bus method exists while app runs:

```sh
gdbus call \
  --session \
  --dest com.ajitid.VoxtralSpeechToText.App \
  --object-path /com/ajitid/VoxtralSpeechToText/App \
  --method com.ajitid.VoxtralSpeechToText.App1.Ping
```

Expected panel behavior:

- Mic icon appears in GNOME top panel.
- `Quit` is enabled while app runs.
- `Type last transcript` is disabled before first transcript.
- After transcription, `Type last transcript` becomes enabled.
- Clicking `Type last transcript` types the previous transcript using existing portal RemoteDesktop path.
- Clicking `Quit` exits the app.
- If app exits, extension disables menu items via bus name watch.

## Risks / notes

- GNOME Shell extensions are shell-version-sensitive. The current metadata supports GNOME 45–50, so keep APIs compatible with those versions.
- `PopupMenu.PopupMenuItem#setSensitive()` is the expected GNOME Shell menu API. If a shell version mismatch appears, adjust to `sensitive` property only after checking runtime logs.
- A custom drawn icon matching the macOS generated tray icon is possible, but initial implementation should use `audio-input-microphone-symbolic` to keep the patch small and avoid asset plumbing.
- The extension is now required for both overlay and Linux panel menu. This is consistent with the existing Linux/GNOME Wayland setup.

## Assumptions

- No AppIndicator/KSNI fallback should be kept for Linux.
- No GTK/GTK4 dependency should be introduced for Linux.
- `tray-icon`/`muda` are fine in principle, but current `tray-icon` Linux tray support is AppIndicator-based, so Linux tray/menu support is GNOME-only via the bundled shell extension.
- macOS keeps existing native tray/menu-bar implementation via `tray-icon`.
