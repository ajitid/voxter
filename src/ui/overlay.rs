use crate::ui::render::OverlayRenderer;
use std::sync::Arc;
use winit::dpi::{LogicalSize, PhysicalPosition, Position};
use winit::event_loop::ActiveEventLoop;
use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowLevel};

#[cfg(target_os = "macos")]
use winit::platform::macos::{WindowAttributesExtMacOS, WindowExtMacOS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayState {
    Hidden,
    Recording,
    RecordingLatch,
    Transcribing,
}

pub struct OverlayController {
    window: Arc<Window>,
    renderer: OverlayRenderer,
    state: OverlayState,
}

impl OverlayController {
    pub fn new(event_loop: &ActiveEventLoop) -> Result<Self, String> {
        let mut attrs = Window::default_attributes()
            .with_title("voxtral overlay")
            .with_visible(false)
            .with_transparent(true)
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(LogicalSize::new(420.0, 96.0));

        #[cfg(target_os = "macos")]
        {
            attrs = attrs.with_has_shadow(false);
        }

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .map_err(|e| format!("Failed to create overlay window: {e}"))?,
        );

        #[cfg(target_os = "macos")]
        {
            window.set_has_shadow(false);
            eprintln!("Overlay window shadow enabled: {}", window.has_shadow());
        }

        if let Err(e) = window.set_cursor_hittest(false) {
            eprintln!("Failed to enable click-through overlay: {e}");
        }

        let renderer = OverlayRenderer::new(&window)?;

        Ok(Self {
            window,
            renderer,
            state: OverlayState::Hidden,
        })
    }

    pub fn window_id(&self) -> winit::window::WindowId {
        self.window.id()
    }

    pub fn update_state(
        &mut self,
        event_loop: &ActiveEventLoop,
        next_state: OverlayState,
    ) -> Result<(), String> {
        if next_state == self.state {
            return Ok(());
        }

        self.state = next_state;

        if next_state == OverlayState::Hidden {
            self.window.set_visible(false);
            return Ok(());
        }

        if let Err(e) = self.position_at_bottom_center(event_loop) {
            self.window.set_visible(false);
            return Err(e);
        }

        let label = match next_state {
            OverlayState::Recording => "recording",
            OverlayState::RecordingLatch => "recording (latch)",
            OverlayState::Transcribing => "transcribing",
            OverlayState::Hidden => "",
        };

        self.renderer.draw_label(label);
        self.window.set_visible(true);
        self.window.request_redraw();
        Ok(())
    }

    pub fn handle_resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        self.renderer.resize(new_size);
        if self.state != OverlayState::Hidden {
            let label = match self.state {
                OverlayState::Recording => "recording",
                OverlayState::RecordingLatch => "recording (latch)",
                OverlayState::Transcribing => "transcribing",
                OverlayState::Hidden => "",
            };
            self.renderer.draw_label(label);
        }
    }

    pub fn redraw(&mut self) -> Result<(), String> {
        self.renderer.render()
    }

    fn position_at_bottom_center(&self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let cursor_pos = current_cursor_position()?;

        let monitor = self
            .find_monitor_for_cursor(event_loop, cursor_pos)
            .ok_or_else(|| {
                format!(
                    "No monitor contains cursor position ({:.1}, {:.1})",
                    cursor_pos.0, cursor_pos.1
                )
            })?;

        let monitor_pos = monitor.position();
        let monitor_size = monitor.size();

        let window_size = self.window.outer_size();

        let x = monitor_pos.x + ((monitor_size.width as i32 - window_size.width as i32) / 2);
        let y = monitor_pos.y + (monitor_size.height as i32 - window_size.height as i32 - 36);

        self.window
            .set_outer_position(Position::Physical(PhysicalPosition::new(x, y)));

        Ok(())
    }

    fn find_monitor_for_cursor(
        &self,
        event_loop: &ActiveEventLoop,
        cursor_pos: (f64, f64),
    ) -> Option<MonitorHandle> {
        let (cx, cy) = cursor_pos;
        for monitor in event_loop.available_monitors() {
            let pos = monitor.position();
            let size = monitor.size();
            let left = pos.x as f64;
            let top = pos.y as f64;
            let right = left + size.width as f64;
            let bottom = top + size.height as f64;
            if cx >= left && cx < right && cy >= top && cy < bottom {
                return Some(monitor);
            }
        }
        None
    }
}

#[cfg(target_os = "macos")]
fn current_cursor_position() -> Result<(f64, f64), String> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| "Failed to create CoreGraphics event source for cursor query".to_string())?;
    let event = CGEvent::new(source)
        .map_err(|_| "Failed to create CoreGraphics event for cursor query".to_string())?;
    let location = event.location();

    Ok((location.x, location.y))
}
