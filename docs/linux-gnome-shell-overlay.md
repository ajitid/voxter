# Linux GNOME Shell overlay

GNOME Wayland does not allow normal application windows to force always-on-top placement, exact positioning, or no-focus mapping. For that reason, Voxtral renders the recording overlay inside GNOME Shell with a small Shell extension instead of using the macOS winit/wgpu overlay path.

## Install

```bash
scripts/install-gnome-shell-extension.sh
```

Then enable/check it:

```bash
gnome-extensions enable voxtral-speech-to-text@ajitid
gnome-extensions info voxtral-speech-to-text@ajitid
busctl --user call com.ajitid.VoxtralSpeechToText.Overlay /com/ajitid/VoxtralSpeechToText/Overlay com.ajitid.VoxtralSpeechToText.Overlay1 Ping
```

For a first local install on GNOME Wayland, GNOME Shell may not notice the newly-created extension directory until the Shell is restarted. X11 users can restart GNOME Shell with `Alt+F2`, `r`, Enter; Wayland users generally need to log out and back in once. After GNOME Shell knows about the extension, normal enable/disable/uninstall operations happen live through `gnome-extensions` / `org.gnome.Shell.Extensions`.

## Runtime behavior

The app sends overlay state and live speech level to the extension over the session D-Bus service:

- `hidden`
- `recording`
- `recording_latch`
- `transcribing`

The extension draws a bottom-center, non-reactive Shell actor so it stays above normal windows without taking focus.

The app intentionally fails startup on GNOME Wayland if the extension is unavailable.
