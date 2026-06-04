# Linux workarounds

## GNOME Wayland overlay uses a Shell extension

The visual recording overlay is rendered by a GNOME Shell extension on GNOME Wayland. A normal winit window cannot reliably be positioned bottom-center, stay above app windows, and avoid focus changes under GNOME Wayland compositor rules.

See [Linux GNOME Shell overlay](linux-gnome-shell-overlay.md) for install and verification commands.

Linux typing still uses XDG RemoteDesktop directly; this is independent of overlay rendering.

## Linux typing uses XDG RemoteDesktop directly

On GNOME Wayland, synthetic keyboard input must go through the XDG RemoteDesktop portal. The app uses portal keysyms directly for Linux auto-typing instead of Enigo's released libei backend.

Why not crates.io Enigo `0.6.1` for Linux typing:

- Enigo's released libei backend can type shifted characters incorrectly on GNOME Wayland.
- For example, `Are there any other ways?` may become `are there any other ways/` because uppercase letters and punctuation such as `?` depend on Shift handling.
- Enigo's unreleased git `xdg_desktop` backend fixed this behavior in testing because it sends keysyms through the XDG RemoteDesktop portal.

The app originally used Enigo from git for that unreleased `xdg_desktop` backend, but now calls the same XDG RemoteDesktop portal APIs directly. This lets the app explicitly close the portal session after typing, so GNOME's screen-sharing/remote-interaction indicator should not stay active longer than needed.

## GNOME Remote Desktop prompt persistence

GNOME shows an “Allow Remote Interaction” / “Share” dialog because synthetic keyboard input on Wayland must be explicitly approved through the XDG RemoteDesktop portal.

The portal supports restore tokens and persistence modes:

- `Application`: permission persists only while the application is running.
- `UntilRevoked`: permission persists until the user explicitly revokes it.

This app requests ashpd's `PersistMode::ExplicitlyRevoked`, which maps to the portal's `UntilRevoked` mode. It stores the returned `restore_token` in XDG state and passes it back on startup so GNOME can restore the previous approval across app restarts.

The token is stored at:

- `$XDG_STATE_HOME/voxtral-speech-to-text/remote-desktop-restore-token`
- fallback: `$HOME/.local/state/voxtral-speech-to-text/remote-desktop-restore-token`

Restore tokens may rotate, so the app overwrites this file with the latest token returned after each successful portal session. Deleting the file can force the app to request a fresh portal prompt, but true revocation should be done from GNOME's permission/privacy UI if available.

The app still closes the live RemoteDesktop session immediately after typing, so GNOME's screen-sharing/remote-interaction indicator should not stay active longer than needed.
