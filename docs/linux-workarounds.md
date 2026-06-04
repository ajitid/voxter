# Linux workarounds

## Enigo from git instead of crates.io

On GNOME Wayland, the crates.io release of Enigo `0.6.1` can type shifted characters incorrectly when using the libei backend. For example, `Are there any other ways?` may become `are there any other ways/` because uppercase letters and punctuation such as `?` depend on Shift handling.

This project therefore uses Enigo from the upstream git `main` branch with the unreleased `xdg_desktop` backend:

```toml
enigo = { git = "https://github.com/enigo-rs/enigo", branch = "main", default-features = false, features = ["xdg_desktop", "smol"] }
```

The `xdg_desktop` backend sends keysyms through the XDG RemoteDesktop portal, which works better for shifted characters on GNOME Wayland. The `smol` feature is only the async runtime required by Enigo for portal DBus calls.

## GNOME Remote Desktop prompt persistence

GNOME shows an “Allow Remote Interaction” / “Share” dialog because synthetic keyboard input on Wayland must be explicitly approved through the XDG RemoteDesktop portal.

The portal supports restore tokens and persistence modes:

- `Application`: permission persists only while the application is running.
- `UntilRevoked`: permission persists until the user explicitly revokes it.

Enigo's current git `xdg_desktop` backend requests `PersistMode::Application`, so this app stores and reuses Enigo's `restore_token` only in memory. That should avoid repeated prompts during one app run, but GNOME may prompt again after restarting the app.

To make permission persist across restarts, Enigo would need to expose the portal persist mode in `Settings`, or this project would need to patch/fork Enigo to request:

```rust
PersistMode::UntilRevoked
```

Then the app should persist the returned `restore_token` to disk and pass it back through `Settings::restore_token` on startup. Restore tokens may rotate, so the app must save the latest token after each successful portal session.
