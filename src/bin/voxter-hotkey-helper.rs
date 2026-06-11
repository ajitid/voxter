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

    eprintln!("voxter-hotkey-helper: listening for Left Control + Alt + Windows + H hotkey events");

    let mut left_control_down = false;
    let mut alt_down = false;
    let mut left_windows_down = false;
    let mut h_down = false;
    let mut hotkey_active = false;

    rdev::listen(move |event| {
        match event.event_type {
            EventType::KeyPress(Key::ControlLeft) => left_control_down = true,
            EventType::KeyRelease(Key::ControlLeft) => left_control_down = false,
            EventType::KeyPress(Key::Alt) => alt_down = true,
            EventType::KeyRelease(Key::Alt) => alt_down = false,
            EventType::KeyPress(Key::MetaLeft) => left_windows_down = true,
            EventType::KeyRelease(Key::MetaLeft) => left_windows_down = false,
            EventType::KeyPress(Key::KeyH) => h_down = true,
            EventType::KeyRelease(Key::KeyH) => h_down = false,
            EventType::KeyPress(Key::Space) => emit_event("space_press"),
            _ => {}
        }

        let chord_down = left_control_down && alt_down && left_windows_down && h_down;
        if chord_down && !hotkey_active {
            hotkey_active = true;
            emit_event("hotkey_press");
        } else if !chord_down && hotkey_active {
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
