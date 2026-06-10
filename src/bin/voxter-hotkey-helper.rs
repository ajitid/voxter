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

    eprintln!("voxter-hotkey-helper: listening for Right Alt / AltGr hotkey events");

    let mut right_alt_down = false;

    rdev::listen(move |event| match event.event_type {
        EventType::KeyPress(Key::AltGr) => {
            if !right_alt_down {
                right_alt_down = true;
                emit_event("right_alt_press");
            }
        }
        EventType::KeyRelease(Key::AltGr) => {
            if right_alt_down {
                right_alt_down = false;
            }
            emit_event("right_alt_release");
        }
        EventType::KeyPress(Key::Space) => emit_event("space_press"),
        _ => {}
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
