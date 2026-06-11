use raqote::{DrawOptions, DrawTarget, PathBuilder, SolidSource, Source, StrokeStyle};

pub struct TrayIconRgba {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn build_status_icon_rgba() -> TrayIconRgba {
    // Render at higher resolution than the menu bar display size so macOS has
    // more detail to downsample from. This noticeably improves sharpness.
    let canvas = 64i32;
    let viewbox = 22.0f32;
    let scale = canvas as f32 / viewbox;
    let s = |value: f32| value * scale;

    let mut dt = DrawTarget::new(canvas, canvas);
    dt.clear(SolidSource::from_unpremultiplied_argb(0, 0, 0, 0));

    let black = Source::Solid(SolidSource::from_unpremultiplied_argb(255, 0, 0, 0));
    let muted = Source::Solid(SolidSource::from_unpremultiplied_argb(145, 0, 0, 0));
    let stroke = StrokeStyle {
        width: s(1.6),
        cap: raqote::LineCap::Round,
        join: raqote::LineJoin::Round,
        ..StrokeStyle::default()
    };
    let inner_stroke = StrokeStyle {
        width: s(1.15),
        cap: raqote::LineCap::Round,
        join: raqote::LineJoin::Round,
        ..StrokeStyle::default()
    };

    // Side-on studio shock-mount silhouette: slightly taller, a bit narrower,
    // and with lighter strokes for a cleaner menu-bar read.
    let mut center_bar = PathBuilder::new();
    center_bar.move_to(s(4.6), s(10.8));
    center_bar.line_to(s(17.4), s(10.8));
    dt.stroke(&center_bar.finish(), &black, &stroke, &DrawOptions::new());

    let mut frame = PathBuilder::new();
    frame.move_to(s(5.8), s(6.1));
    frame.line_to(s(4.7), s(10.8));
    frame.line_to(s(5.8), s(15.5));
    frame.move_to(s(16.2), s(6.1));
    frame.line_to(s(17.3), s(10.8));
    frame.line_to(s(16.2), s(15.5));
    dt.stroke(&frame.finish(), &black, &stroke, &DrawOptions::new());

    let mut anchors = PathBuilder::new();
    anchors.move_to(s(6.7), s(7.0));
    anchors.line_to(s(5.8), s(5.6));
    anchors.move_to(s(15.3), s(7.0));
    anchors.line_to(s(16.2), s(5.6));
    anchors.move_to(s(6.7), s(14.6));
    anchors.line_to(s(5.8), s(16.0));
    anchors.move_to(s(15.3), s(14.6));
    anchors.line_to(s(16.2), s(16.0));
    dt.stroke(
        &anchors.finish(),
        &muted,
        &inner_stroke,
        &DrawOptions::new(),
    );

    let mut suspension = PathBuilder::new();
    suspension.move_to(s(6.0), s(6.3));
    suspension.line_to(s(11.0), s(10.8));
    suspension.line_to(s(16.0), s(6.3));
    suspension.move_to(s(6.0), s(15.3));
    suspension.line_to(s(11.0), s(10.8));
    suspension.line_to(s(16.0), s(15.3));
    suspension.move_to(s(7.8), s(6.2));
    suspension.line_to(s(14.2), s(15.4));
    suspension.move_to(s(14.2), s(6.2));
    suspension.line_to(s(7.8), s(15.4));
    dt.stroke(
        &suspension.finish(),
        &muted,
        &inner_stroke,
        &DrawOptions::new(),
    );

    let mut rgba = Vec::with_capacity((canvas * canvas * 4) as usize);
    for pixel in dt.get_data() {
        let alpha = ((pixel >> 24) & 0xFF) as u8;
        rgba.extend_from_slice(&[0, 0, 0, alpha]);
    }

    TrayIconRgba {
        width: canvas as u32,
        height: canvas as u32,
        rgba,
    }
}
