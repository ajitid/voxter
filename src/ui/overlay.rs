use crate::SpeechVizState;
use crate::ui::render::OverlayRenderer;
use crate::ui::state::OverlayState;
use std::sync::Arc;
use std::time::Instant;
use winit::dpi::{LogicalSize, PhysicalPosition, Position};
use winit::event_loop::ActiveEventLoop;
#[cfg(target_os = "macos")]
use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowLevel};

#[cfg(target_os = "macos")]
use winit::platform::macos::{WindowAttributesExtMacOS, WindowExtMacOS};

pub struct OverlayController {
    window: Arc<Window>,
    renderer: OverlayRenderer,
    state: OverlayState,
    visible_since: Option<Instant>,
}

impl OverlayController {
    pub fn new(
        event_loop: &ActiveEventLoop,
        speech_viz: Arc<SpeechVizState>,
    ) -> Result<Self, String> {
        let attrs = Window::default_attributes()
            .with_title("voxtral overlay")
            .with_visible(false)
            .with_transparent(true)
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(LogicalSize::new(420.0, 96.0));

        #[cfg(target_os = "macos")]
        let attrs = attrs.with_has_shadow(false);

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

        let renderer = OverlayRenderer::new(&window, speech_viz)?;

        Ok(Self {
            window,
            renderer,
            state: OverlayState::Hidden,
            visible_since: None,
        })
    }

    pub fn window_id(&self) -> winit::window::WindowId {
        self.window.id()
    }

    pub fn window(&self) -> &Window {
        self.window.as_ref()
    }

    pub fn is_visible(&self) -> bool {
        self.state != OverlayState::Hidden
    }

    pub fn update_state(
        &mut self,
        event_loop: &ActiveEventLoop,
        next_state: OverlayState,
    ) -> Result<(), String> {
        if next_state == self.state {
            return Ok(());
        }

        let was_visible = self.is_visible();
        self.state = next_state;

        if !self.is_visible() {
            self.visible_since = None;
            self.window.set_visible(false);
            return Ok(());
        }

        if let Err(e) = self.position_at_bottom_center(event_loop) {
            self.window.set_visible(false);
            return Err(e);
        }

        if !was_visible {
            self.visible_since = Some(Instant::now());
        }

        self.window.set_visible(true);
        if !was_visible {
            self.window.request_redraw();
        }

        Ok(())
    }

    pub fn handle_resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>, scale_factor: f64) {
        self.renderer.resize(new_size, scale_factor);
        if self.is_visible() {
            self.window.request_redraw();
        }
    }

    pub fn request_redraw(&self) {
        if self.is_visible() {
            self.window.request_redraw();
        }
    }

    pub fn redraw(&mut self) -> Result<(), String> {
        if !self.is_visible() {
            return Ok(());
        }

        let now = Instant::now();
        let started_at = self.visible_since.unwrap_or(now);
        self.renderer.draw_frame(self.state, now, started_at);
        self.renderer.render()
    }

    fn position_at_bottom_center(&self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        let monitor = {
            let cursor_pos = current_cursor_position()?;
            self.find_monitor_for_cursor(event_loop, cursor_pos)
                .ok_or_else(|| {
                    format!(
                        "No monitor contains cursor position ({:.1}, {:.1})",
                        cursor_pos.0, cursor_pos.1
                    )
                })?
        };

        #[cfg(target_os = "linux")]
        let monitor = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next())
            .ok_or_else(|| "No monitor available for overlay positioning".to_string())?;

        let monitor_pos = monitor.position();
        let monitor_size = monitor.size();

        let window_size = self.window.outer_size();

        let x = monitor_pos.x + ((monitor_size.width as i32 - window_size.width as i32) / 2);
        let y = monitor_pos.y + (monitor_size.height as i32 - window_size.height as i32);

        self.window
            .set_outer_position(Position::Physical(PhysicalPosition::new(x, y)));

        Ok(())
    }

    #[cfg(target_os = "macos")]
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
