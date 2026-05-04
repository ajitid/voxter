# Plan: make Linux/GNOME portal hotkeys latch-only

## Goal

Change the Linux XDG Desktop Portal GlobalShortcuts backend from HOLD/LATCH semantics to a single latch/toggle shortcut because GNOME/Mutter release detection for chorded global shortcuts is unreliable for hold-to-record UX.

Desired Linux behavior:

- Register only one portal shortcut id: `vstt_record`.
- Press `vstt_record` once: start recording.
- Press `vstt_record` again: stop recording and process transcription.
- Do not register or use `vstt_switch_to_latch` on Linux.
- Keep macOS behavior unchanged: Right Option HOLD mode and Space-to-LATCH continue to work.

## User decisions locked

- Linux/Wayland-GNOME should be latch-only.
- macOS HOLD mode should remain unchanged.
- No backwards compatibility is required for old shortcut ids or old Linux behavior.

## Why this change

References checked:

- XDG GlobalShortcuts docs: `Activated` and `Deactivated` signals exist, but backend behavior is compositor-specific.
  - https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html
- GNOME portal backend forwards GNOME Shell `AcceleratorActivated` / `AcceleratorDeactivated`.
  - https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/raw/main/src/globalshortcuts.c
- Mutter release handling matches the original accelerator chord on key release. For chords such as `<Super>C`, if the modifier is released before the non-modifier key, the release event may no longer match the original chord, so `Deactivated` is not reliable enough for hold-to-record.
  - https://github.com/GNOME/mutter/raw/main/src/core/keybindings.c

## Files to edit

- `src/main.rs`
- `docs/linux-hotkeys.md`
- `CLAUDE.md` if any Linux mode guidance still mentions hold/latch split.
- `.pi/todos/linux-gnome-latch-only.md` during implementation.

## Implementation details

### 1. Add implementation todo file

Create `.pi/todos/linux-gnome-latch-only.md` with phases:

```md
# Linux/GNOME latch-only todos

## Phase 1: Code changes
- [ ] Remove Linux portal registration for `vstt_switch_to_latch`
- [ ] Make Linux `vstt_record` activation toggle start/stop
- [ ] Ignore Linux `Deactivated` events for recording control
- [ ] Keep macOS hotkey behavior unchanged

## Phase 2: Docs
- [ ] Update Linux hotkey docs to latch-only
- [ ] Update CLAUDE.md Linux behavior notes

## Phase 3: Verification
- [ ] cargo fmt
- [ ] cargo check
- [ ] cargo clippy
- [ ] Optional: run via terminalcp and manually test `vstt_record` press to start/stop
```

### 2. Keep shared control messages but make Linux send only toggle

Current relevant types in `src/main.rs`:

```rust
enum RecordingMode {
    Hold,
    Latch,
}

enum ControlMsg {
    StopHold,
    SinglePress,
    SwitchToLatch,
    Quit,
}
```

Do not remove `RecordingMode::Hold`, `ControlMsg::StopHold`, or `ControlMsg::SwitchToLatch` globally because macOS uses them:

- macOS listener sends `SinglePress`, `StopHold`, `SwitchToLatch` around `src/main.rs:1190-1210`.
- `App::handle_control` uses these messages for macOS hold/latch semantics.

Linux should simply stop sending `StopHold` and `SwitchToLatch`.

### 3. Change Linux startup text

Current Linux text around `main()` says:

```rust
println!("  HOLD: Hold the portal-configured record shortcut, release to transcribe");
println!(
    "  LATCH: Activate the portal-configured latch shortcut while in HOLD mode, then activate the record shortcut again to stop"
);
```

Replace Linux block with latch-only wording:

```rust
println!("  LATCH: Press the portal-configured record shortcut to start recording; press it again to stop and transcribe");
```

Optionally rename heading from `Recording modes:` to still be generic. No need to change macOS text.

### 4. Change Linux portal shortcut registration

Current `run_linux_global_shortcuts_listener()` has:

```rust
let shortcuts = [
    NewShortcut::new("vstt_record", "Hold to record / release to transcribe"),
    NewShortcut::new(
        "vstt_switch_to_latch",
        "Switch current hold recording to latch mode",
    ),
];
```

Replace with one shortcut:

```rust
let shortcuts = [NewShortcut::new(
    "vstt_record",
    "Start/stop recording and transcribe",
)];
```

### 5. Change existing shortcut check

Current code checks both ids:

```rust
let has_record = existing_shortcuts.iter().any(|id| id == "vstt_record");
let has_latch = existing_shortcuts
    .iter()
    .any(|id| id == "vstt_switch_to_latch");

if has_record && has_latch {
    println!("Using existing portal global shortcut bindings");
} else {
    ... bind_shortcuts ...
}
```

Replace with only:

```rust
let has_record = existing_shortcuts.iter().any(|id| id == "vstt_record");

if has_record {
    println!("Using existing portal global shortcut binding");
} else {
    ... bind_shortcuts ...
}
```

Question for implementation: because no backwards compatibility is desired, do not attempt to clean or migrate an existing `vstt_switch_to_latch` binding here. The user can run `--unbind` before rebinding if needed.

### 6. Change Linux event loop

Current Linux listener merges `Activated` and `Deactivated` and sends:

- `vstt_record` activated → `ControlMsg::SinglePress`
- `vstt_switch_to_latch` activated → `ControlMsg::SwitchToLatch`
- `vstt_record` deactivated → `ControlMsg::StopHold`

For latch-only Linux, use only activated events. Simplest implementation:

```rust
let mut activated = portal
    .receive_activated()
    .await
    .map_err(|e| e.to_string())?;

while let Some(event) = activated.next().await {
    if event.shortcut_id() == "vstt_record" {
        let _ = proxy.send_event(AppEvent::Control(ControlMsg::SinglePress));
    }
}
```

Do not subscribe to `receive_deactivated()` for Linux unless keeping it only for debug logging. Prefer not to keep defensive extras.

### 7. Adjust control handling so Linux toggle starts in LATCH mode

Current `ControlMsg::SinglePress` logic:

```rust
ControlMsg::SinglePress => {
    if self.audio_manager.recorder.is_recording()
        && self.audio_manager.mode == RecordingMode::Latch
    {
        stop_recording(...)
    } else if !self.audio_manager.recorder.is_recording()
        && let Err(e) = self.audio_manager.start_recording(RecordingMode::Hold)
    {
        ...
    } else {
        send OverlayState::Recording
    }
}
```

This starts `SinglePress` in `Hold`, which worked for macOS because release stops hold. For Linux latch-only, `SinglePress` must start in `Latch`.

Recommended clean approach: add a platform-specific helper in `App`:

```rust
fn single_press_start_mode() -> RecordingMode {
    #[cfg(target_os = "linux")]
    {
        RecordingMode::Latch
    }
    #[cfg(not(target_os = "linux"))]
    {
        RecordingMode::Hold
    }
}
```

Then replace:

```rust
self.audio_manager.start_recording(RecordingMode::Hold)
```

with:

```rust
self.audio_manager.start_recording(Self::single_press_start_mode())
```

This preserves macOS behavior and makes Linux toggle stop work because `mode == RecordingMode::Latch` will be true after the first Linux press.

Overlay behavior: the existing `else` branch sends `OverlayState::Recording`. For Linux latch-only, after starting in `Latch`, prefer showing latch overlay:

Option A (recommended): after successful start, send overlay based on mode:

```rust
let mode = Self::single_press_start_mode();
if let Err(e) = self.audio_manager.start_recording(mode) { ... }
else {
    let overlay = match mode {
        RecordingMode::Hold => OverlayState::Recording,
        RecordingMode::Latch => OverlayState::RecordingLatch,
    };
    let _ = self.proxy.send_event(AppEvent::Overlay(overlay));
}
```

This may require restructuring the `else if let Err` chain for clarity.

### 8. Optional cleanup: Linux no longer needs SwitchToLatch description

Do not remove `switch_to_latch_mode()` because macOS uses Space during hold.

But Linux docs and portal registration should no longer mention `vstt_switch_to_latch`.

### 9. Update `--unbind`

Current Linux unbind ids:

```rust
const SHORTCUT_IDS: &[&str] = &["vstt_record", "vstt_switch_to_latch"];
```

Because user said no backwards compatibility is needed, and Linux will no longer add `vstt_switch_to_latch`, change to:

```rust
const SHORTCUT_IDS: &[&str] = &["vstt_record"];
```

Caveat: this means `cargo run -- --unbind` will not clear old `vstt_switch_to_latch` entries from previous local builds. That is intentional per user preference. If old entries cause binding confusion, manually clear with:

```sh
dconf reset -f /org/gnome/settings-daemon/global-shortcuts/surface-transient/
```

or temporarily include old id during local cleanup before finalizing. Ask before adding any temporary migration code.

Update no-bindings message from:

```rust
No GNOME global shortcut bindings found for vstt_record/vstt_switch_to_latch.
```

to:

```rust
No GNOME global shortcut bindings found for vstt_record.
```

### 10. Update docs

#### `docs/linux-hotkeys.md`

Change setup text from two actions:

```md
Bind the actions to whatever shortcuts you prefer:

- "Hold to record / release to transcribe"
- "Switch current hold recording to latch mode"
```

To one action:

```md
Bind the action to whatever shortcut you prefer:

- "Start/stop recording and transcribe"

Press the shortcut once to start recording. Press it again to stop recording and transcribe.
```

Update clearing section:

```md
This clears GNOME's stored binding for the app action `vstt_record`.
```

Add a short note explaining why no hold mode on GNOME:

```md
GNOME portal shortcuts are used as activation events. This app does not use hold-to-record on Linux because release/deactivation for chorded shortcuts can be unreliable depending on release order. macOS still supports hold mode.
```

Do not recommend raw `/dev/input/event*`, `input` group, or evdev.

#### `CLAUDE.md`

Update Linux docs:

- Replace any statement implying Linux has HOLD mode.
- Keep architecture note that Linux uses XDG Desktop Portal GlobalShortcuts.
- Update Recording Modes section to say:

```md
- **macOS HOLD mode**: Hold Right Option/Alt to record, release to transcribe
- **macOS LATCH mode**: Press Space during HOLD to switch; press hotkey again to stop
- **Linux portal latch mode**: Press configured global shortcut once to start, again to stop/process
```

### 11. Verification

Run:

```sh
cargo fmt
cargo check
cargo clippy
```

Optional manual check with terminalcp:

```sh
terminalcp start voxtral-run "cargo run"
terminalcp stdout voxtral-run 80
# manually press configured GNOME shortcut once: expect recording starts in LATCH mode
# press same shortcut again: expect recording stops and processing begins
terminalcp stdout voxtral-run 120
terminalcp stop voxtral-run
```

Expected logs:

- `Using existing portal global shortcut binding` or GNOME binding dialog appears.
- First press: `Started recording in LATCH mode (...)`.
- Second press: recording stops, VAD/transcription flow begins.

## Risks / notes

- Existing GNOME binding storage may still contain `vstt_switch_to_latch` from previous builds. Since no backwards compatibility is requested, the code will not clean it. If it interferes with testing, clear the whole current app section manually with dconf or run an explicitly temporary cleanup before final code.
- If `bind_shortcuts()` errors continue, that is separate from hold/release behavior. It likely relates to portal session/app id/binding state and should be debugged with exact stderr plus `dconf dump /org/gnome/settings-daemon/global-shortcuts/`.
- This plan intentionally keeps the portal-only security model and does not reintroduce evdev/raw keyboard access.
