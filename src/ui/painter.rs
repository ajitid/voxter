use crate::SpeechVizState;
use crate::ui::state::OverlayState;
#[cfg(target_os = "macos")]
use raqote::BlendMode;
use raqote::{
    Color, DrawOptions, DrawTarget, Gradient, GradientStop, LineCap, PathBuilder, Point,
    SolidSource, Source, Spread, StrokeStyle,
};
use std::sync::Arc;
use std::time::Instant;
use winit::dpi::{LogicalSize, PhysicalSize};

pub struct OverlayPainter {
    dt: DrawTarget,
    physical_size: PhysicalSize<u32>,
    logical_size: LogicalSize<f32>,
    scale_factor: f64,
    speech_viz: Arc<SpeechVizState>,
    last_state: OverlayState,
    state_started_at: Instant,
}

impl OverlayPainter {
    pub fn new(
        physical_size: PhysicalSize<u32>,
        scale_factor: f64,
        speech_viz: Arc<SpeechVizState>,
    ) -> Self {
        let physical_size =
            PhysicalSize::new(physical_size.width.max(1), physical_size.height.max(1));
        Self {
            dt: DrawTarget::new(physical_size.width as i32, physical_size.height as i32),
            logical_size: physical_size.to_logical::<f32>(scale_factor),
            physical_size,
            scale_factor,
            speech_viz,
            last_state: OverlayState::Hidden,
            state_started_at: Instant::now(),
        }
    }

    pub fn resize(&mut self, physical_size: PhysicalSize<u32>, scale_factor: f64) {
        if physical_size.width == 0 || physical_size.height == 0 {
            return;
        }
        self.physical_size = physical_size;
        self.scale_factor = scale_factor;
        self.logical_size = physical_size.to_logical::<f32>(scale_factor);
        self.dt = DrawTarget::new(physical_size.width as i32, physical_size.height as i32);
    }

    pub fn data_u8(&self) -> &[u8] {
        self.dt.get_data_u8()
    }

    #[cfg(target_os = "macos")]
    pub fn physical_size(&self) -> PhysicalSize<u32> {
        self.physical_size
    }

    fn draw_polyline_source(&mut self, points: &[Point], width: f32, source: &Source<'_>) {
        if points.len() < 2 {
            return;
        }

        let mut pb = PathBuilder::new();
        pb.move_to(points[0].x, points[0].y);
        for point in &points[1..] {
            pb.line_to(point.x, point.y);
        }
        let path = pb.finish();

        self.dt.stroke(
            &path,
            source,
            &StrokeStyle {
                width: width.max(0.8),
                cap: LineCap::Round,
                ..StrokeStyle::default()
            },
            &DrawOptions::new(),
        );
    }

    fn soft_premium_gradient(alpha_mult: f32) -> Gradient {
        let a = ((235.0 * alpha_mult.clamp(0.0, 1.0)).round()).clamp(0.0, 255.0) as u8;
        Gradient {
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: Color::new(a, 0xFF, 0x4D, 0x4D), // red
                },
                GradientStop {
                    position: 0.5,
                    color: Color::new(a, 0xFF, 0x9F, 0x43), // orange
                },
                GradientStop {
                    position: 1.0,
                    color: Color::new(a, 0xFF, 0x5F, 0xA2), // pink
                },
            ],
        }
    }

    fn arc_gradient_source(left_x: f32, right_x: f32, y: f32, alpha_mult: f32) -> Source<'static> {
        Source::new_linear_gradient(
            Self::soft_premium_gradient(alpha_mult),
            Point::new(left_x, y),
            Point::new(right_x, y),
            Spread::Pad,
        )
    }

    #[cfg(target_os = "macos")]
    fn draw_latch_lock_icon(&mut self, center: Point) {
        let lock_pink = SolidSource::from_unpremultiplied_argb(235, 0xFF, 0x5F, 0xA2);

        let body_w = 9.8;
        let body_h = 8.0;
        let body_left = center.x - (body_w * 0.5);
        let body_top = center.y - 0.7;

        let mut body_pb = PathBuilder::new();
        body_pb.rect(body_left, body_top, body_w, body_h);
        let body_path = body_pb.finish();
        self.dt
            .fill(&body_path, &Source::Solid(lock_pink), &DrawOptions::new());

        let mut shackle_pb = PathBuilder::new();
        shackle_pb.arc(
            center.x,
            body_top + 0.6,
            3.5,
            std::f32::consts::PI,
            std::f32::consts::PI,
        );
        let shackle_path = shackle_pb.finish();
        self.dt.stroke(
            &shackle_path,
            &Source::Solid(lock_pink),
            &StrokeStyle {
                width: 2.0,
                cap: LineCap::Round,
                ..StrokeStyle::default()
            },
            &DrawOptions::new(),
        );

        let mut keyhole_pb = PathBuilder::new();
        keyhole_pb.arc(center.x, body_top + 3.2, 1.0, 0.0, std::f32::consts::TAU);
        keyhole_pb.rect(center.x - 0.55, body_top + 3.8, 1.1, 1.9);
        let keyhole_path = keyhole_pb.finish();
        self.dt.fill(
            &keyhole_path,
            &Source::Solid(SolidSource::from_unpremultiplied_argb(255, 0, 0, 0)),
            &DrawOptions {
                blend_mode: BlendMode::Clear,
                ..DrawOptions::new()
            },
        );
    }

    fn arc_points_from_sagitta(
        cx: f32,
        y_base: f32,
        half_chord: f32,
        sagitta: f32,
        samples: usize,
    ) -> Vec<Point> {
        let s = sagitta.max(0.5);
        let a = half_chord.max(1.0);
        let radius = ((a * a) + (s * s)) / (2.0 * s);
        let cy = y_base + (radius - s);

        let mut points = Vec::with_capacity(samples + 1);
        for i in 0..=samples {
            let t = i as f32 / samples as f32;
            let x = cx - a + (2.0 * a * t);
            let dx = x - cx;
            let y = cy - ((radius * radius - dx * dx).max(0.0)).sqrt();
            points.push(Point::new(x, y));
        }
        points
    }

    fn draw_sine_squares(&mut self, cx: f32, y_base: f32, elapsed: f32) {
        let n_squares = 4;
        let square_size = 6.0;
        let spacing = 8.0;
        let total_width = (n_squares as f32 * square_size) + ((n_squares - 1) as f32 * spacing);
        let start_x = cx - (total_width * 0.5);

        let colors = [
            SolidSource::from_unpremultiplied_argb(235, 0xFF, 0x4D, 0x4D), // Red
            SolidSource::from_unpremultiplied_argb(235, 0xFF, 0x9F, 0x43), // Orange/Yellow
            SolidSource::from_unpremultiplied_argb(235, 0xFF, 0x5F, 0xA2), // Pink
        ];

        let amplitude = 6.0;
        let frequency = 9.0;
        let phase_step = 0.8;

        for i in 0..n_squares {
            let x = start_x + (i as f32 * (square_size + spacing));
            let phase = i as f32 * phase_step;
            let y_offset = (elapsed * frequency + phase).sin() * amplitude;
            let y = y_base - 10.0 + y_offset;

            let color = &colors[i % colors.len()];
            let mut pb = PathBuilder::new();
            pb.rect(x, y - (square_size * 0.5), square_size, square_size);
            let path = pb.finish();
            self.dt
                .fill(&path, &Source::Solid(*color), &DrawOptions::new());
        }
    }

    pub fn draw_frame(&mut self, state: OverlayState, now: Instant, _started_at: Instant) {
        let width = self.logical_size.width.max(1.0);
        let height = self.logical_size.height.max(1.0);

        self.dt
            .clear(SolidSource::from_unpremultiplied_argb(0, 0, 0, 0));
        self.dt.set_transform(&raqote::Transform::scale(
            self.scale_factor as f32,
            self.scale_factor as f32,
        ));

        if state != self.last_state {
            self.last_state = state;
            self.state_started_at = now;
        }

        let state_elapsed = now
            .saturating_duration_since(self.state_started_at)
            .as_secs_f32();

        let live_speech_energy = if self.speech_viz.is_active() {
            self.speech_viz.level()
        } else {
            0.0
        }
        .clamp(0.0, 1.0);
        let display_energy = live_speech_energy.powf(0.72);

        let cx = width * 0.5;
        let half_chord = (width * 0.26)
            .min(width * 0.42)
            .max((width * 0.18).max(48.0));
        let y_base = height - 14.0;
        let sagitta_min = 1.8;
        let sagitta_range = height * 0.42;
        let sample_count = 56usize;

        let stroke_width = 3.8;

        match state {
            OverlayState::Recording => {
                let sagitta = sagitta_min + (display_energy * sagitta_range);
                let arc_points =
                    Self::arc_points_from_sagitta(cx, y_base, half_chord, sagitta, sample_count);
                let arc_source =
                    Self::arc_gradient_source(cx - half_chord, cx + half_chord, y_base, 1.0);
                self.draw_polyline_source(&arc_points, stroke_width, &arc_source);
            }
            #[cfg(target_os = "macos")]
            OverlayState::RecordingLatch => {
                let sagitta = sagitta_min + (display_energy * sagitta_range);
                let arc_points =
                    Self::arc_points_from_sagitta(cx, y_base, half_chord, sagitta, sample_count);
                let arc_source =
                    Self::arc_gradient_source(cx - half_chord, cx + half_chord, y_base, 1.0);
                self.draw_polyline_source(&arc_points, stroke_width, &arc_source);

                const LOCK_ICON_GAP_FROM_ARC_END: f32 = 20.0;
                const LOCK_ICON_MIN_MARGIN_RIGHT: f32 = 22.0;
                const LOCK_ICON_BASELINE_OFFSET: f32 = 4.5;
                let lock_x = (cx + half_chord + LOCK_ICON_GAP_FROM_ARC_END)
                    .min(width - LOCK_ICON_MIN_MARGIN_RIGHT);
                let lock_center = Point::new(lock_x, y_base - LOCK_ICON_BASELINE_OFFSET);
                self.draw_latch_lock_icon(lock_center);
            }
            OverlayState::Transcribing => {
                self.draw_sine_squares(cx, y_base, state_elapsed);
            }
            OverlayState::Hidden => {}
        }

        self.dt.set_transform(&raqote::Transform::identity());
    }
}
