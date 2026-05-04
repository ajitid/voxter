# Linux hotkey permissions

The Linux backend uses the XDG Desktop Portal GlobalShortcuts interface. This does not require membership in the `input` group and does not grant raw `/dev/input/event*` access.

## Requirements

- A desktop/portal backend with `org.freedesktop.portal.GlobalShortcuts` support.
- `xdg-desktop-portal` and the desktop-specific backend installed/running.

## Setup

Run the app. On first launch, the desktop may show a shortcut binding/permission dialog. Bind the action to whatever shortcut you prefer:

- "Start/stop recording and transcribe"

Press the shortcut once to start recording. Press it again to stop recording and transcribe.

When editing a shortcut on GNOME, you may see an "Allow inhibiting shortcuts" dialog for `org.gnome.Settings.GlobalShortcutsProvider`. Click **Allow**. This lets GNOME Settings temporarily capture the keys you press while editing the shortcut. If normal desktop shortcuts need to be restored while this capture mode is active, press `Super+Escape`.

## Clearing GNOME bindings

To remove the GNOME global shortcut bindings registered by this app:

```sh
cargo run -- --unbind
```

This clears GNOME's stored binding for the app action `vstt_record`.

## Why no hold mode on GNOME?

GNOME portal shortcuts are used as activation events. This app does not use hold-to-record on Linux because release/deactivation for chorded shortcuts can be unreliable depending on release order. macOS still supports hold mode.

## Why not `input` group?

Adding your user to `input` lets any process running as your user read raw keyboard events, which is keylogging-capable. It is not recommended for normal desktop use.

## Unsupported portals

If your desktop/portal backend does not support `org.freedesktop.portal.GlobalShortcuts`, Linux hotkeys will fail loudly. There is intentionally no raw evdev fallback.
