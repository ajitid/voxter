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

    eprintln!("voxter-hotkey-helper: listening for Super+C hotkey events");

    let mut super_down = false;
    let mut c_down = false;
    let mut combo_active = false;

    rdev::listen(move |event| match event.event_type {
        EventType::KeyPress(Key::MetaLeft | Key::MetaRight) => {
            super_down = true;
            maybe_emit_super_c_press(super_down, c_down, &mut combo_active);
        }
        EventType::KeyRelease(Key::MetaLeft | Key::MetaRight) => {
            super_down = false;
            if combo_active {
                combo_active = false;
                emit_event("super_c_release");
            }
        }
        EventType::KeyPress(Key::KeyC) => {
            c_down = true;
            maybe_emit_super_c_press(super_down, c_down, &mut combo_active);
        }
        EventType::KeyRelease(Key::KeyC) => {
            c_down = false;
            if combo_active {
                combo_active = false;
                emit_event("super_c_release");
            }
        }
        EventType::KeyPress(Key::Space) => emit_event("space_press"),
        _ => {}
    })
    .map_err(|error| format!("global hotkey listener failed: {error:?}"))?;

    fn maybe_emit_super_c_press(super_down: bool, c_down: bool, combo_active: &mut bool) {
        if super_down && c_down && !*combo_active {
            *combo_active = true;
            emit_event("super_c_press");
        }
    }

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
