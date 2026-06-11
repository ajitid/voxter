#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux_main() {
        eprintln!("voxter-hotkey-helper: {error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
fn linux_main() -> Result<(), String> {
    use rdev::{EventType, Key};
    use std::io::{self, Write};

    eprintln!(
        "voxter-hotkey-helper: listening for Left Control + Left Alt + Left Windows + J hotkey events"
    );

    let mut left_control_down = false;
    let mut left_alt_down = false;
    let mut left_windows_down = false;
    let mut j_down = false;
    let mut hotkey_active = false;

    rdev::listen(move |event| {
        match event.event_type {
            EventType::KeyPress(Key::ControlLeft) => left_control_down = true,
            EventType::KeyRelease(Key::ControlLeft) => left_control_down = false,
            // rdev reports the left Alt key as Key::Alt; right Alt is Key::AltGr.
            EventType::KeyPress(Key::Alt) => left_alt_down = true,
            EventType::KeyRelease(Key::Alt) => left_alt_down = false,
            EventType::KeyPress(Key::MetaLeft) => left_windows_down = true,
            EventType::KeyRelease(Key::MetaLeft) => left_windows_down = false,
            EventType::KeyPress(Key::KeyJ) => j_down = true,
            EventType::KeyRelease(Key::KeyJ) => j_down = false,
            _ => {}
        }

        let record_chord_down = left_control_down && left_alt_down && left_windows_down && j_down;
        if record_chord_down && !hotkey_active {
            hotkey_active = true;
            emit_event("hotkey_press");
        } else if !record_chord_down && hotkey_active {
            hotkey_active = false;
            emit_event("hotkey_release");
        }
    })
    .map_err(|error| format!("global hotkey listener failed: {error:?}"))?;

    fn emit_event(event: &str) {
        let mut stdout = io::stdout().lock();
        if let Err(error) = writeln!(stdout, "{{\"event\":\"{event}\"}}") {
            eprintln!("voxter-hotkey-helper: failed to write event to stdout: {error}");
            return;
        }
        if let Err(error) = stdout.flush() {
            eprintln!("voxter-hotkey-helper: failed to flush stdout: {error}");
        }
    }

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("voxter-hotkey-helper is only supported on Linux");
    std::process::exit(1);
}
