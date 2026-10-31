//! Draws annotations on the editor canvas with egui, for instant feedback.
//! The exported image comes from `guide_render` instead; "Preview export"
//! shows that exact result.

use eframe::egui::{
    pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind,
    TextureHandle,
};
use guide_core::{arrow_head_len, Annotation, Kind, Point, RectF};

use super::{props::to_color32, view::View};

fn stroke(a: &Annotation, v: &View) -> Stroke {
    Stroke::new((a.style.width * v.zoom).max(1.0), to_color32(a.style.color))
}

/// `patch` is the already pixelated/blurred picture for an `Obscure` annotation.
pub fn paint_annotation(
    p: &Painter,
    v: &View,
    a: &Annotation,
    patch: Option<(&TextureHandle, RectF)>,
) {
    let st = stroke(a, v);
    let pts = |points: &[Point]| -> Vec<Pos2> { points.iter().map(|q| v.to_screen(*q)).collect() };
    match &a.kind {
        Kind::Pen { points } | Kind::Highlighter { points } => match points.as_slice() {
            [] => {}
            [only] => {
                p.circle_filled(v.to_screen(*only), st.width / 2.0, st.color);
            }
            _ => {
                p.add(Shape::line(pts(points), st));
                // Round caps and joins, which Shape::line does not draw.
                // Only for opaque strokes: translucent circles would stack up into beads.
                if st.width > 3.0 && st.color.a() == 255 {
                    for q in pts(points) {
                        p.circle_filled(q, st.width / 2.0, st.color);
                    }
                }
            }
        },
        Kind::Line { from, to } => {
            p.line_segment([v.to_screen(*from), v.to_screen(*to)], st);
        }
        Kind::Arrow { from, to } => paint_arrow(p, v, a, *from, *to),
        Kind::Rectangle { rect } => {
            let r = v.screen_rect(*rect);
            if let Some(f) = a.style.fill {
                p.rect_filled(r, 0.0, to_color32(f));
            }
            if a.style.width > 0.0 {
                p.rect_stroke(r, 0.0, st, StrokeKind::Middle);
            }
        }
        Kind::Ellipse { rect } => {
            let r = v.screen_rect(*rect);
            if let Some(f) = a.style.fill {
                p.add(Shape::ellipse_filled(
                    r.center(),
                    r.size() / 2.0,
                    to_color32(f),
                ));
            }
            if a.style.width > 0.0 {
                p.add(Shape::ellipse_stroke(r.center(), r.size() / 2.0, st));
            }
        }
        Kind::Text {
            at,
            text,
            size,
            bold,
        } => {
            let pos = v.to_screen(*at);
            let font = FontId::proportional((*size * v.zoom).max(1.0));
            let color = to_color32(a.style.color);
            if let Some(bg) = a.style.fill {
                let (w, h) = guide_render::measure_text(text, *size);
                let pad = (*size * 0.25).max(2.0) * v.zoom;
                let r = Rect::from_min_size(pos, vec2(w * v.zoom, h * v.zoom)).expand(pad);
                p.rect_filled(r, 0.0, to_color32(bg));
            }
            p.text(pos, Align2::LEFT_TOP, text, font.clone(), color);
            if *bold {
                p.text(
                    pos + vec2((*size / 18.0).max(0.6) * v.zoom, 0.0),
                    Align2::LEFT_TOP,
                    text,
                    font,
                    color,
                );
            }
        }
        Kind::Marker {
            center,
            number,
            radius,
        } => {
            let c = v.to_screen(*center);
            let ring = (radius / 8.0).max(1.5);
            p.circle_filled(c, (radius + ring) * v.zoom, Color32::WHITE);
            p.circle_filled(c, radius * v.zoom, to_color32(a.style.color));
            p.text(
                c,
                Align2::CENTER_CENTER,
                number.to_string(),
                FontId::proportional(radius * 1.2 * v.zoom),
                Color32::WHITE,
            );
        }
        Kind::Obscure { rect, .. } => {
            let r = v.screen_rect(*rect);
            if let Some((tex, region)) = patch {
                let sr = v.screen_rect(region);
                p.image(
                    tex.id(),
                    sr,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                p.rect_filled(r, 0.0, Color32::from_black_alpha(120));
            }
            p.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, Color32::LIGHT_BLUE),
                StrokeKind::Middle,
            );
        }
    }
}

fn paint_arrow(p: &Painter, v: &View, a: &Annotation, from: Point, to: Point) {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let len = (dx * dx + dy * dy).sqrt();
    let st = stroke(a, v);
    if len < 0.5 {
        p.circle_filled(v.to_screen(from), st.width / 2.0, st.color);
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = arrow_head_len(a.style.width).min(len);
    let base = Point::new(to.x - ux * head * 0.8, to.y - uy * head * 0.8);
    p.line_segment([v.to_screen(from), v.to_screen(base)], st);
    let (px, py) = (-uy * head * 0.45, ux * head * 0.45);
    let tri = vec![
        v.to_screen(to),
        v.to_screen(Point::new(base.x + px, base.y + py)),
        v.to_screen(Point::new(base.x - px, base.y - py)),
    ];
    p.add(Shape::convex_polygon(tri, st.color, Stroke::NONE));
}

/// White squares with a blue border at the eight resize handles of `r` (screen coordinates).
pub fn paint_handles(p: &Painter, v: &View, r: RectF) {
    for h in super::view::Handle::ALL {
        let c = v.to_screen(h.position(r));
        let sq = Rect::from_center_size(c, vec2(8.0, 8.0));
        p.rect_filled(sq, 0.0, Color32::WHITE);
        p.rect_stroke(
            sq,
            0.0,
            Stroke::new(1.5, Color32::from_rgb(40, 120, 255)),
            StrokeKind::Middle,
        );
    }
    p.rect_stroke(
        v.screen_rect(r),
        0.0,
        Stroke::new(1.0, Color32::from_rgb(40, 120, 255)),
        StrokeKind::Middle,
    );
}

/// Dim everything outside the crop and outline it.
pub fn paint_crop(p: &Painter, v: &View, image: (u32, u32), crop: RectF) {
    let all = v.screen_rect(RectF {
        x: 0.0,
        y: 0.0,
        w: image.0 as f32,
        h: image.1 as f32,
    });
    let c = v.screen_rect(crop);
    let dim = Color32::from_black_alpha(140);
    p.rect_filled(
        Rect::from_min_max(all.min, pos2(all.max.x, c.min.y)),
        0.0,
        dim,
    );
    p.rect_filled(
        Rect::from_min_max(pos2(all.min.x, c.max.y), all.max),
        0.0,
        dim,
    );
    p.rect_filled(
        Rect::from_min_max(pos2(all.min.x, c.min.y), pos2(c.min.x, c.max.y)),
        0.0,
        dim,
    );
    p.rect_filled(
        Rect::from_min_max(pos2(c.max.x, c.min.y), pos2(all.max.x, c.max.y)),
        0.0,
        dim,
    );
    p.rect_stroke(
        c,
        0.0,
        Stroke::new(1.5, Color32::YELLOW),
        StrokeKind::Outside,
    );
}
