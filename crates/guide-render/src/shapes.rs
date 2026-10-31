use guide_core::{arrow_head_len, Annotation, Kind, Point, RectF, Rgba, Style};
use tiny_skia::{
    BlendMode, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke,
    Transform,
};

fn paint(c: Rgba, blend: BlendMode) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.0[0], c.0[1], c.0[2], c.0[3]);
    p.anti_alias = true;
    p.blend_mode = blend;
    p
}

fn stroke(width: f32) -> Stroke {
    Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

fn to_rect(r: &RectF) -> Option<Rect> {
    Rect::from_xywh(r.x, r.y, r.w, r.h)
}

fn polyline(points: &[Point]) -> Option<Path> {
    let (first, rest) = points.split_first()?;
    let mut pb = PathBuilder::new();
    pb.move_to(first.x, first.y);
    for p in rest {
        pb.line_to(p.x, p.y);
    }
    pb.finish()
}

fn stroke_path(pm: &mut Pixmap, path: &Path, style: &Style, blend: BlendMode) {
    pm.stroke_path(
        path,
        &paint(style.color, blend),
        &stroke(style.width),
        Transform::identity(),
        None,
    );
}

fn fill_path(pm: &mut Pixmap, path: &Path, color: Rgba) {
    pm.fill_path(
        path,
        &paint(color, BlendMode::SourceOver),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn dot(pm: &mut Pixmap, at: Point, radius: f32, color: Rgba, blend: BlendMode) {
    if let Some(path) = PathBuilder::from_circle(at.x, at.y, radius.max(0.5)) {
        pm.fill_path(
            &path,
            &paint(color, blend),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// Draws pen, highlighter, line, arrow, rectangle and ellipse.
pub fn draw(pm: &mut Pixmap, a: &Annotation) {
    let style = &a.style;
    match &a.kind {
        Kind::Pen { points } => freehand(pm, points, style, BlendMode::SourceOver),
        // Multiply keeps dark text readable under the highlight, like a real marker.
        Kind::Highlighter { points } => freehand(pm, points, style, BlendMode::Multiply),
        Kind::Line { from, to } => {
            if let Some(path) = polyline(&[*from, *to]) {
                stroke_path(pm, &path, style, BlendMode::SourceOver);
            }
        }
        Kind::Arrow { from, to } => arrow(pm, *from, *to, style),
        Kind::Rectangle { rect } => {
            if let Some(r) = to_rect(rect) {
                let path = PathBuilder::from_rect(r);
                if let Some(fill) = style.fill {
                    fill_path(pm, &path, fill);
                }
                if style.width > 0.0 {
                    stroke_path(pm, &path, style, BlendMode::SourceOver);
                }
            }
        }
        Kind::Ellipse { rect } => {
            if let Some(path) = to_rect(rect).and_then(PathBuilder::from_oval) {
                if let Some(fill) = style.fill {
                    fill_path(pm, &path, fill);
                }
                if style.width > 0.0 {
                    stroke_path(pm, &path, style, BlendMode::SourceOver);
                }
            }
        }
        Kind::Text { .. } | Kind::Marker { .. } | Kind::Obscure { .. } => {}
    }
}

fn freehand(pm: &mut Pixmap, points: &[Point], style: &Style, blend: BlendMode) {
    match points {
        [] => {}
        [only] => dot(pm, *only, style.width / 2.0, style.color, blend),
        _ => {
            if let Some(path) = polyline(points) {
                stroke_path(pm, &path, style, blend);
            }
        }
    }
}

fn arrow(pm: &mut Pixmap, from: Point, to: Point, style: &Style) {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.5 {
        dot(
            pm,
            from,
            style.width / 2.0,
            style.color,
            BlendMode::SourceOver,
        );
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = arrow_head_len(style.width).min(len);
    let base = Point::new(to.x - ux * head * 0.8, to.y - uy * head * 0.8);
    if let Some(path) = polyline(&[from, base]) {
        stroke_path(pm, &path, style, BlendMode::SourceOver);
    }
    let (px, py) = (-uy * head * 0.45, ux * head * 0.45);
    let mut pb = PathBuilder::new();
    pb.move_to(to.x, to.y);
    pb.line_to(base.x + px, base.y + py);
    pb.line_to(base.x - px, base.y - py);
    pb.close();
    if let Some(path) = pb.finish() {
        fill_path(pm, &path, style.color);
    }
}
