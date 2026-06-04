# Linux workarounds

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

This app currently requests `PersistMode::Application`, stores the returned `restore_token` in memory, and passes it to the next typing session. That should avoid repeated prompts during one app run, but GNOME may prompt again after restarting the app.

To make permission persist across restarts, the app should request:

```rust
PersistMode::UntilRevoked
```

Then it should persist the returned `restore_token` to disk and pass it back on startup. Restore tokens may rotate, so the app must save the latest token after each successful portal session.
