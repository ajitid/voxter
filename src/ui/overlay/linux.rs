use crate::SpeechVizState;
use crate::ui::overlay::OverlayState;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::event_loop::ActiveEventLoop;
use zbus::blocking;

const BUS_NAME: &str = "com.ajitid.VoxtralSpeechToText.Overlay";
const OBJECT_PATH: &str = "/com/ajitid/VoxtralSpeechToText/Overlay";
const INTERFACE: &str = "com.ajitid.VoxtralSpeechToText.Overlay1";
const MIN_SEND_INTERVAL: Duration = Duration::from_millis(33);
const MIN_LEVEL_DELTA: f32 = 0.008;

const EXTENSION_REQUIRED_ERROR: &str = "GNOME Shell overlay extension is required on GNOME Wayland but is not available. Install with scripts/install-gnome-shell-extension.sh, enable it, then log out/in.";

pub struct OverlayController {
    speech_viz: Arc<SpeechVizState>,
    connection: blocking::Connection,
    state: OverlayState,
    last_sent_at: Option<Instant>,
    last_sent_level: f32,
}

impl OverlayController {
    pub fn new(
        _event_loop: &ActiveEventLoop,
        speech_viz: Arc<SpeechVizState>,
    ) -> Result<Self, String> {
        let connection = connect_and_ping()?;
        Ok(Self {
            speech_viz,
            connection,
            state: OverlayState::Hidden,
            last_sent_at: None,
            last_sent_level: 0.0,
        })
    }

    pub fn is_visible(&self) -> bool {
        self.state != OverlayState::Hidden
    }

    pub fn update_state(
        &mut self,
        _event_loop: &ActiveEventLoop,
        next_state: OverlayState,
    ) -> Result<(), String> {
        if next_state == self.state {
            if self.is_visible() {
                return self.send_overlay(true);
            }
            return Ok(());
        }

        self.state = next_state;
        self.send_overlay(true)
    }

    pub fn request_redraw(&mut self) {
        if self.is_visible()
            && let Err(e) = self.send_overlay(false)
        {
            eprintln!("GNOME Shell overlay update failed: {e}");
        }
    }

    fn send_overlay(&mut self, force: bool) -> Result<(), String> {
        let level = current_level(&self.speech_viz, self.state);
        let now = Instant::now();

        if !force {
            if self
                .last_sent_at
                .is_some_and(|last| now.duration_since(last) < MIN_SEND_INTERVAL)
            {
                return Ok(());
            }

            if (level - self.last_sent_level).abs() < MIN_LEVEL_DELTA
                && self.state != OverlayState::Transcribing
            {
                return Ok(());
            }
        }

        let proxy = overlay_proxy(&self.connection)?;
        proxy
            .call::<_, _, ()>("SetOverlay", &(state_name(self.state), level as f64))
            .map_err(|e| format!("GNOME Shell overlay update failed: {e}"))?;

        self.last_sent_at = Some(now);
        self.last_sent_level = level;
        Ok(())
    }
}

pub fn check_overlay_available() -> Result<(), String> {
    connect_and_ping().map(|_| ())
}

fn connect_and_ping() -> Result<blocking::Connection, String> {
    let connection = blocking::Connection::session()
        .map_err(|e| format!("{EXTENSION_REQUIRED_ERROR}\nSession D-Bus connection error: {e}"))?;
    {
        let proxy =
            overlay_proxy(&connection).map_err(|e| format!("{EXTENSION_REQUIRED_ERROR}\n{e}"))?;
        let version = proxy
            .call::<_, _, String>("Ping", &())
            .map_err(|e| format!("{EXTENSION_REQUIRED_ERROR}\nPing failed: {e}"))?;
        println!("GNOME Shell overlay extension available (version {version})");
    }
    Ok(connection)
}

fn overlay_proxy(connection: &blocking::Connection) -> Result<blocking::Proxy<'_>, String> {
    blocking::Proxy::new(connection, BUS_NAME, OBJECT_PATH, INTERFACE)
        .map_err(|e| format!("GNOME Shell overlay D-Bus proxy error: {e}"))
}

fn current_level(speech_viz: &SpeechVizState, state: OverlayState) -> f32 {
    if (state == OverlayState::Recording || state == OverlayState::RecordingLatch)
        && speech_viz.is_active()
    {
        speech_viz.level().clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn state_name(state: OverlayState) -> &'static str {
    match state {
        OverlayState::Hidden => "hidden",
        OverlayState::Recording => "recording",
        OverlayState::RecordingLatch => "recording_latch",
        OverlayState::Transcribing => "transcribing",
    }
}
