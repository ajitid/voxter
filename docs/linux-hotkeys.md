# Linux hotkey permissions

On GNOME Wayland, the bundled GNOME Shell extension captures the recording shortcut and calls the app over D-Bus. This does not require membership in the `input` group, does not grant raw `/dev/input/event*` access, and intentionally does not use the XDG Desktop Portal GlobalShortcuts interface for recording.

## Requirements

- GNOME Shell with the bundled extension installed/enabled.
- `xdg-desktop-portal` and the GNOME backend installed/running for app identity and RemoteDesktop typing.
- A matching installed desktop file: `com.ajitid.VoxtralSpeechToText.desktop`.

## Setup

For development runs, build the binary, install/update the user desktop file, and install/update the GNOME Shell extension:

```sh
cargo build
scripts/install-linux-desktop-file.sh
scripts/install-gnome-shell-extension.sh
```

By default the desktop-file script points to `target/debug/voxtral-speech-to-text`. To use another binary:

```sh
scripts/install-linux-desktop-file.sh --app-path ./target/release/voxtral-speech-to-text
```

The extension provides the overlay, panel menu, and recording shortcut. The default shortcut is `<Super>c`.

Press the shortcut once to start recording. Press it again to stop recording and transcribe.

## Changing the shortcut

There is no preferences UI yet. Change the shortcut with GSettings:

```sh
gsettings set com.ajitid.VoxtralSpeechToText.Extension record-shortcut "['<Super>c']"
gsettings set com.ajitid.VoxtralSpeechToText.Extension record-shortcut "['<Alt>space']"
```

## Why no hold mode on GNOME?

GNOME extension shortcuts are activation events. This app does not use hold-to-record on Linux; Linux uses latch/toggle mode. macOS still supports hold mode.

## Why not `input` group?

Adding your user to `input` lets any process running as your user read raw keyboard events, which is keylogging-capable. It is not recommended for normal desktop use.
