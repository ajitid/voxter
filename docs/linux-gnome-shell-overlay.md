# Linux GNOME Shell overlay and shortcut

GNOME Wayland does not allow normal application windows to force always-on-top placement, exact positioning, or no-focus mapping. For that reason, Voxtral renders the recording overlay inside GNOME Shell with a small Shell extension instead of using the macOS winit/wgpu overlay path.

The same extension also provides the panel menu and captures the Linux recording shortcut. The default shortcut is `<Super>c` and uses latch behavior: press once to start recording, press again to stop/transcribe.

## Install

```bash
scripts/install-gnome-shell-extension.sh
```

Then enable/check it:

```bash
gnome-extensions enable voxtral-speech-to-text@ajitid
gnome-extensions info voxtral-speech-to-text@ajitid
gsettings get com.ajitid.VoxtralSpeechToText.Extension record-shortcut
busctl --user call com.ajitid.VoxtralSpeechToText.Overlay /com/ajitid/VoxtralSpeechToText/Overlay com.ajitid.VoxtralSpeechToText.Overlay1 Ping
```

For a first local install on GNOME Wayland, GNOME Shell may not notice the newly-created extension directory until the Shell is restarted. X11 users can restart GNOME Shell with `Alt+F2`, `r`, Enter; Wayland users generally need to log out and back in once. After GNOME Shell knows about the extension, normal enable/disable/uninstall operations happen live through `gnome-extensions` / `org.gnome.Shell.Extensions`.

## Changing the shortcut

There is no preferences UI yet. Change the shortcut with GSettings:

```sh
gsettings set com.ajitid.VoxtralSpeechToText.Extension record-shortcut "['<Super>c']"
gsettings set com.ajitid.VoxtralSpeechToText.Extension record-shortcut "['<Alt>space']"
```

## Runtime behavior

The app sends overlay state and live speech level to the extension over the session D-Bus service:

- `hidden`
- `recording`
- `recording_latch`
- `transcribing`

The extension draws a bottom-center, non-reactive Shell actor so it stays above normal windows without taking focus.

The extension calls the app's `ToggleRecording`, `TypeLastTranscript`, and `Quit` D-Bus methods for the shortcut and panel menu.

The app intentionally fails startup on GNOME Wayland if the extension is unavailable.
