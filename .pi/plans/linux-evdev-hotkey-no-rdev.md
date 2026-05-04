# Plan: Linux/GNOME hotkeys without `rdev`

## Goal

Make Linux builds avoid installing/building `rdev`, while preserving the current Right Option/Right Alt hold-to-record, Space-to-latch, Right Alt-to-stop behavior on CachyOS GNOME Wayland.

User preference locked: use direct evdev input on Linux, accepting `/dev/input/event*` permissions / `input` group requirements.

## Findings / references

- Last commit `e33e5c6 wayland instructions` only changed `CLAUDE.md` to suggest `cargo run --features rdev/wayland`; this is misleading.
- `rdev` README says Linux `listen` uses X11 APIs and “will not work in Wayland”: <https://github.com/Narsil/rdev>
- Current machine: `GNOME Shell 50.2`, `XDG_SESSION_TYPE=wayland`, `xdg-desktop-portal-gnome 50.0-1.1`.
- GNOME 50 has `org.freedesktop.portal.GlobalShortcuts` with `Activated`/`Deactivated`, but freedesktop shortcut syntax is accelerator-style (`CTRL+ALT+Return`), not a good fit for pure Right Alt as a hold key.
- `hotkey-listener` is Linux-evdev and target-gates `rdev` to macOS, but its public `Key` enum only supports F1-F12/Insert/Pause/ScrollLock, so it does **not** preserve Right Alt/Space as-is. Use `evdev` directly instead.
- `evdev` docs: `/dev/input/eventX`, `Device::open`, `supported_keys()`, `fetch_events()`, `EventSummary::Key`, `KeyCode::KEY_RIGHTALT`, `KeyCode::KEY_SPACE`: <https://docs.rs/evdev/latest/evdev/>

## Architecture

Add platform-specific hotkey modules:

- macOS: keep current `rdev` listener.
- Linux: new direct evdev listener that watches keyboard devices and emits existing `ControlMsg`s.
- Other OS: compile error or no support.

The existing app control semantics stay unchanged:

- Right Alt press -> `ControlMsg::SinglePress`
- Right Alt release -> `ControlMsg::StopHold`
- Space press while recording in HOLD mode -> `ControlMsg::SwitchToLatch`

Important: evdev reads all hardware keyboard events. We will only inspect Right Alt and Space, but permissions still grant broad input access.

## Patch spec

### 1. `Cargo.toml`

Move `rdev` out of global dependencies and add Linux-only `evdev`.

Replace:

```toml
rdev = { git = "https://github.com/Narsil/rdev", rev = "c14f2dc5c8100a96c5d7e3013de59d6aa0b9eae2" }
```

with:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
rdev = { git = "https://github.com/Narsil/rdev", rev = "c14f2dc5c8100a96c5d7e3013de59d6aa0b9eae2" }
core-graphics = "0.24.0"

[target.'cfg(target_os = "linux")'.dependencies]
evdev = "0.13.2"
```

and remove the existing duplicate macOS section containing only `core-graphics`.

### 2. `src/main.rs`: remove Linux compile error

Replace:

```rust
#[cfg(not(target_os = "macos"))]
compile_error!("This build currently supports macOS only (rdev + on-demand cursor query).");
```

with:

```rust
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("This build currently supports macOS and Linux only.");
```

### 3. `src/main.rs`: split hotkey listener

Replace the current single `spawn_hotkey_listener` with platform-specific implementations.

#### macOS implementation

Keep existing logic, but annotate it:

```rust
#[cfg(target_os = "macos")]
fn spawn_hotkey_listener(proxy: EventLoopProxy<AppEvent>) {
    thread::spawn(move || {
        use rdev::{EventType, Key, set_is_main_thread};
        // existing callback unchanged
    });
}
```

#### Linux implementation

Add:

```rust
#[cfg(target_os = "linux")]
fn spawn_hotkey_listener(proxy: EventLoopProxy<AppEvent>) {
    thread::spawn(move || {
        if let Err(e) = run_linux_evdev_hotkey_listener(proxy) {
            eprintln!("Linux evdev hotkey listener error: {e}");
            eprintln!("Hint: add your user to the input group or install a udev rule granting read access to /dev/input/event* keyboards, then log out/in.");
        }
    });
}
```

Add helper:

```rust
#[cfg(target_os = "linux")]
fn run_linux_evdev_hotkey_listener(proxy: EventLoopProxy<AppEvent>) -> Result<(), String> {
    use evdev::{Device, EventSummary, KeyCode};
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel::<ControlMsg>();
    let mut opened = 0usize;

    for (path, device) in evdev::enumerate() {
        let Some(keys) = device.supported_keys() else { continue; };
        if !(keys.contains(KeyCode::KEY_RIGHTALT) || keys.contains(KeyCode::KEY_SPACE)) {
            continue;
        }

        opened += 1;
        let tx = tx.clone();
        thread::spawn(move || {
            let mut device = match Device::open(&path) {
                Ok(device) => device,
                Err(e) => {
                    eprintln!("Failed to open input device {}: {e}", path.display());
                    return;
                }
            };

            loop {
                match device.fetch_events() {
                    Ok(events) => {
                        for event in events {
                            match event.destructure() {
                                EventSummary::Key(_, KeyCode::KEY_RIGHTALT, 1) => {
                                    let _ = tx.send(ControlMsg::SinglePress);
                                }
                                EventSummary::Key(_, KeyCode::KEY_RIGHTALT, 0) => {
                                    let _ = tx.send(ControlMsg::StopHold);
                                }
                                EventSummary::Key(_, KeyCode::KEY_SPACE, 1) => {
                                    let _ = tx.send(ControlMsg::SwitchToLatch);
                                }
                                // Ignore key-repeat value 2 and all other events.
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Input device {} read error: {e}; stopping device listener", path.display());
                        break;
                    }
                }
            }
        });
    }

    if opened == 0 {
        return Err("no readable keyboard devices with Right Alt/Space found".to_string());
    }

    drop(tx);
    while let Ok(msg) = rx.recv() {
        let _ = proxy.send_event(AppEvent::Control(msg));
    }

    Ok(())
}
```

Notes for implementer:

- If `evdev::enumerate()` returns already-open devices and opening `path` again is unnecessary, simplify by moving the enumerated `device` into the per-device thread instead. Verify against the actual `evdev` API during implementation.
- This first version does not hotplug-rescan keyboards. Add a periodic rescan later only if needed.
- Do not use `EVIOCGRAB`; we only observe events and must not steal keyboard input from GNOME/apps.

### 4. `CLAUDE.md`

Update development commands / architecture notes:

- Remove: `cargo run --features rdev/wayland`.
- Add Linux note: `cargo run` on GNOME Wayland uses evdev and needs read access to `/dev/input/event*` keyboard devices.
- Replace “rdev thread” with “platform hotkey listener thread: rdev on macOS, evdev on Linux”.

### 5. Optional permission helper docs

Add a small `docs/linux-evdev-hotkeys.md` or README section:

```md
# Linux hotkey permissions

The Linux hotkey backend reads evdev keyboard events from `/dev/input/event*`.

Quick setup on CachyOS/Arch:

```sh
sudo usermod -aG input "$USER"
# log out and log back in
```

Verify:

```sh
id | grep input
ls -l /dev/input/event*
```
```

Mention this grants broad input read permission. If desired later, implement a tighter udev rule for only keyboard devices.

## Verification

Run:

```sh
cargo update
cargo check
cargo fmt
cargo clippy
```

Linux-specific checks:

```sh
cargo tree -i rdev --target x86_64-unknown-linux-gnu
# expected: no rdev dependency for Linux

cargo tree -i evdev --target x86_64-unknown-linux-gnu
# expected: evdev present
```

Runtime smoke test on GNOME Wayland:

1. Ensure user has input permissions; log out/in if group changed.
2. `cargo run`
3. Hold Right Alt: overlay enters recording.
4. Release Right Alt: recording stops/transcribes.
5. Hold Right Alt, press Space: switches to latch.
6. Press Right Alt again: latch recording stops.

## Risks / follow-ups

- `input` group access is effectively keylogging-capable. This is the tradeoff selected to preserve pure Right Alt hold semantics on Wayland.
- No keyboard hotplug handling in the first patch. If keyboards are plugged after app start, restart app or add a rescan loop.
- Multi-keyboard duplicate events are unlikely for a single physical keypress, but if duplicate press events appear, add an `AtomicBool right_alt_down` debounce in the Linux listener.
