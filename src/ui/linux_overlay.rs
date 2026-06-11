use crate::SpeechVizState;
use crate::ui::painter::OverlayPainter;
use crate::ui::state::OverlayState;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;
use wayland_client::{
    Connection, EventQueue, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};
use winit::dpi::PhysicalSize;

const OVERLAY_WIDTH: u32 = 420;
const OVERLAY_HEIGHT: u32 = 96;

pub struct LinuxOverlayController {
    event_queue: EventQueue<LayerApp>,
    qh: QueueHandle<LayerApp>,
    app: LayerApp,
}

impl LinuxOverlayController {
    pub fn new(speech_viz: Arc<SpeechVizState>) -> Result<Self, String> {
        let conn =
            Connection::connect_to_env().map_err(|e| format!("Wayland connection failed: {e}"))?;
        let (globals, event_queue) = registry_queue_init(&conn)
            .map_err(|e| format!("Wayland registry initialization failed: {e}"))?;
        let qh = event_queue.handle();

        let compositor = CompositorState::bind(&globals, &qh)
            .map_err(|e| format!("wl_compositor is not available: {e}"))?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .map_err(|e| format!("zwlr_layer_shell_v1 is not available: {e}"))?;
        let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm is not available: {e}"))?;
        let pool = SlotPool::new((OVERLAY_WIDTH * OVERLAY_HEIGHT * 4) as usize, &shm)
            .map_err(|e| format!("Failed to create Wayland SHM pool: {e}"))?;

        let app = LayerApp {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            shm,
            compositor,
            layer_shell,
            pool,
            layer: None,
            painter: OverlayPainter::new(
                PhysicalSize::new(OVERLAY_WIDTH, OVERLAY_HEIGHT),
                1.0,
                speech_viz,
            ),
            state: OverlayState::Hidden,
            visible_since: None,
            first_configure: false,
            configured_width: OVERLAY_WIDTH,
            configured_height: OVERLAY_HEIGHT,
            needs_redraw: false,
            closed: false,
        };

        Ok(Self {
            event_queue,
            qh,
            app,
        })
    }

    pub fn update_state(&mut self, next_state: OverlayState) -> Result<(), String> {
        if next_state == self.app.state {
            return Ok(());
        }

        self.app.state = next_state;
        if next_state == OverlayState::Hidden {
            self.app.visible_since = None;
            self.app.layer = None;
            self.app.needs_redraw = false;
            self.event_queue
                .flush()
                .map_err(|e| format!("Wayland flush failed: {e}"))?;
            return Ok(());
        }

        if self.app.layer.is_none() {
            self.app.create_layer(&self.qh);
            self.event_queue
                .roundtrip(&mut self.app)
                .map_err(|e| format!("Wayland dispatch failed: {e}"))?;
        }
        if self.app.visible_since.is_none() {
            self.app.visible_since = Some(Instant::now());
        }
        self.app.needs_redraw = true;
        self.redraw_if_visible()
    }

    pub fn is_visible(&self) -> bool {
        self.app.state != OverlayState::Hidden && self.app.layer.is_some()
    }

    pub fn dispatch_pending(&mut self) -> Result<(), String> {
        self.event_queue
            .dispatch_pending(&mut self.app)
            .map_err(|e| format!("Wayland dispatch failed: {e}"))?;
        if self.app.closed {
            self.app.layer = None;
            self.app.state = OverlayState::Hidden;
            self.app.closed = false;
        }
        Ok(())
    }

    pub fn redraw_if_visible(&mut self) -> Result<(), String> {
        if !self.is_visible() {
            return Ok(());
        }
        self.app.draw(&self.qh)?;
        self.event_queue
            .flush()
            .map_err(|e| format!("Wayland flush failed: {e}"))
    }
}

struct LayerApp {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    compositor: CompositorState,
    layer_shell: LayerShell,
    pool: SlotPool,
    layer: Option<LayerSurface>,
    painter: OverlayPainter,
    state: OverlayState,
    visible_since: Option<Instant>,
    first_configure: bool,
    configured_width: u32,
    configured_height: u32,
    needs_redraw: bool,
    closed: bool,
}

impl LayerApp {
    fn create_layer(&mut self, qh: &QueueHandle<Self>) {
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("voxter-overlay"),
            None,
        );
        layer.set_anchor(Anchor::BOTTOM);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(OVERLAY_WIDTH, OVERLAY_HEIGHT);
        layer.set_margin(0, 0, 0, 0);
        layer.set_exclusive_zone(0);
        layer.commit();

        self.layer = Some(layer);
        self.first_configure = true;
        self.configured_width = OVERLAY_WIDTH;
        self.configured_height = OVERLAY_HEIGHT;
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) -> Result<(), String> {
        let Some(layer) = self.layer.as_ref() else {
            return Ok(());
        };
        let width = self.configured_width.max(1);
        let height = self.configured_height.max(1);
        let stride = width as i32 * 4;

        self.painter.resize(PhysicalSize::new(width, height), 1.0);
        self.painter.draw_frame(
            self.state,
            Instant::now(),
            self.visible_since.unwrap_or_else(Instant::now),
        );

        let (buffer, canvas) = self
            .pool
            .create_buffer(
                width as i32,
                height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .map_err(|e| format!("Failed to create Wayland SHM buffer: {e}"))?;
        let pixels = self.painter.data_u8();
        if canvas.len() < pixels.len() {
            return Err(format!(
                "Wayland SHM canvas too small: canvas={} bytes, pixels={} bytes",
                canvas.len(),
                pixels.len()
            ));
        }
        canvas.fill(0);
        canvas[..pixels.len()].copy_from_slice(pixels);

        layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        let _ = qh;
        buffer
            .attach_to(layer.wl_surface())
            .map_err(|e| format!("Failed to attach Wayland buffer: {e}"))?;
        layer.commit();
        self.needs_redraw = false;
        Ok(())
    }
}

impl CompositorHandler for LayerApp {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for LayerApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for LayerApp {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.closed = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.configured_width =
            NonZeroU32::new(configure.new_size.0).map_or(OVERLAY_WIDTH, NonZeroU32::get);
        self.configured_height =
            NonZeroU32::new(configure.new_size.1).map_or(OVERLAY_HEIGHT, NonZeroU32::get);
        self.first_configure = false;
        self.needs_redraw = true;
    }
}

impl ShmHandler for LayerApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(LayerApp);
delegate_output!(LayerApp);
delegate_shm!(LayerApp);
delegate_layer!(LayerApp);
delegate_registry!(LayerApp);

impl ProvidesRegistryState for LayerApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}
