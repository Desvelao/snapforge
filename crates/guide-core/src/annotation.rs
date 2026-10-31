//! Editable annotation model.
//!
//! **Coordinate system:** every coordinate is in *pixels of the original
//! screenshot*, origin at its top-left, y pointing down. Zoom, pan, window
//! size and HiDPI scale only affect how the editor draws these numbers, never
//! the numbers themselves, so annotations stay aligned on any display.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// Axis-aligned rectangle with non-negative `w` and `h`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    /// Rectangle spanning two corners given in any order.
    pub fn from_points(a: Point, b: Point) -> Self {
        Self {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            w: (a.x - b.x).abs(),
            h: (a.y - b.y).abs(),
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }

    pub fn inflate(&self, d: f32) -> Self {
        Self {
            x: self.x - d,
            y: self.y - d,
            w: (self.w + 2.0 * d).max(0.0),
            h: (self.h + 2.0 * d).max(0.0),
        }
    }

    fn of_points(points: &[Point]) -> Self {
        let mut it = points.iter();
        let Some(first) = it.next() else {
            return Self {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            };
        };
        it.fold(
            Self {
                x: first.x,
                y: first.y,
                w: 0.0,
                h: 0.0,
            },
            |r, p| {
                Self::from_points(
                    Point::new(r.x.min(p.x), r.y.min(p.y)),
                    Point::new(r.right().max(p.x), r.bottom().max(p.y)),
                )
            },
        )
    }
}

/// Whole-pixel rectangle (used for the crop).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const RED: Rgba = Rgba([230, 40, 40, 255]);
    pub const WHITE: Rgba = Rgba([255, 255, 255, 255]);
    pub const HIGHLIGHT_YELLOW: Rgba = Rgba([255, 235, 0, 255]);
}

/// Stable identity of an annotation inside one document. Never reused, even
/// after the annotation is deleted or an add is undone.
/// Ids from this value up are refused when loading: a damaged or hand-edited file could otherwise push
/// the next id over `u64::MAX`, where it would wrap around and collide with an existing one.
pub(crate) const MAX_ID: u64 = u64::MAX / 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AnnotationId(pub u64);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Style {
    /// Outline / stroke / text colour.
    pub color: Rgba,
    /// Stroke width in image pixels.
    pub width: f32,
    /// Interior colour for rectangles and ellipses; background for text.
    pub fill: Option<Rgba>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color: Rgba::RED,
            width: 4.0,
            fill: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObscureMode {
    Pixelate,
    Blur,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Kind {
    Pen {
        points: Vec<Point>,
    },
    Highlighter {
        points: Vec<Point>,
    },
    Line {
        from: Point,
        to: Point,
    },
    Arrow {
        from: Point,
        to: Point,
    },
    Rectangle {
        rect: RectF,
    },
    Ellipse {
        rect: RectF,
    },
    /// `at` is the top-left corner; `size` is the font height in pixels.
    Text {
        at: Point,
        text: String,
        size: f32,
        bold: bool,
    },
    Marker {
        center: Point,
        number: u32,
        radius: f32,
    },
    /// Replaces the pixels below it with a pixelated or blurred version when rendered.
    Obscure {
        rect: RectF,
        mode: ObscureMode,
        strength: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: AnnotationId,
    pub kind: Kind,
    pub style: Style,
}

/// Length of an arrow head for a given stroke width.
pub fn arrow_head_len(width: f32) -> f32 {
    (width * 4.0).max(14.0)
}

/// Approximate size of a text annotation; the renderer's own measurement is exact.
fn estimate_text_size(text: &str, size: f32) -> (f32, f32) {
    let lines = text.split('\n');
    let (mut max_chars, mut count) = (0usize, 0usize);
    for l in lines {
        max_chars = max_chars.max(l.chars().count());
        count += 1;
    }
    (max_chars as f32 * size * 0.55, count as f32 * size * 1.25)
}

fn dist_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
    };
    ((p.x - (a.x + t * dx)).powi(2) + (p.y - (a.y + t * dy)).powi(2)).sqrt()
}

fn dist_to_polyline(p: Point, pts: &[Point]) -> f32 {
    match pts {
        [] => f32::INFINITY,
        [only] => ((p.x - only.x).powi(2) + (p.y - only.y).powi(2)).sqrt(),
        _ => pts
            .windows(2)
            .map(|w| dist_to_segment(p, w[0], w[1]))
            .fold(f32::INFINITY, f32::min),
    }
}

impl Annotation {
    /// Bounds of the shape itself, without stroke thickness. Resize handles
    /// and [`Annotation::transform_to`] work on this.
    pub fn geometry_bounds(&self) -> RectF {
        match &self.kind {
            Kind::Pen { points } | Kind::Highlighter { points } => RectF::of_points(points),
            Kind::Line { from, to } | Kind::Arrow { from, to } => RectF::from_points(*from, *to),
            Kind::Rectangle { rect } | Kind::Ellipse { rect } | Kind::Obscure { rect, .. } => *rect,
            Kind::Text { at, text, size, .. } => {
                let (w, h) = estimate_text_size(text, *size);
                RectF {
                    x: at.x,
                    y: at.y,
                    w,
                    h,
                }
            }
            Kind::Marker { center, radius, .. } => RectF {
                x: center.x - radius,
                y: center.y - radius,
                w: radius * 2.0,
                h: radius * 2.0,
            },
        }
    }

    /// Everything the annotation can paint: geometry plus stroke thickness and arrow head.
    pub fn bounds(&self) -> RectF {
        let g = self.geometry_bounds();
        match &self.kind {
            Kind::Arrow { .. } => {
                g.inflate(self.style.width / 2.0 + arrow_head_len(self.style.width) / 2.0)
            }
            Kind::Text { .. } | Kind::Marker { .. } | Kind::Obscure { .. } => g,
            _ => g.inflate(self.style.width / 2.0),
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.map_points(|p| Point::new(p.x + dx, p.y + dy));
    }

    /// Stretches the annotation so the geometry that filled `from` fills `to`
    /// (used by the resize handles). Text grows with the height ratio and
    /// markers with the smaller ratio. A zero-sized `from` axis only translates.
    pub fn transform_to(&mut self, from: RectF, to: RectF) {
        let sx = if from.w > 0.0 { to.w / from.w } else { 1.0 };
        let sy = if from.h > 0.0 { to.h / from.h } else { 1.0 };
        let map = |p: Point| Point::new(to.x + (p.x - from.x) * sx, to.y + (p.y - from.y) * sy);
        self.map_points(map);
        match &mut self.kind {
            Kind::Text { size, .. } => *size = (*size * sy).max(1.0),
            Kind::Marker { radius, .. } => *radius = (*radius * sx.min(sy)).max(1.0),
            _ => {}
        }
    }

    fn map_points(&mut self, f: impl Fn(Point) -> Point) {
        match &mut self.kind {
            Kind::Pen { points } | Kind::Highlighter { points } => {
                points.iter_mut().for_each(|p| *p = f(*p));
            }
            Kind::Line { from, to } | Kind::Arrow { from, to } => {
                (*from, *to) = (f(*from), f(*to));
            }
            Kind::Rectangle { rect } | Kind::Ellipse { rect } | Kind::Obscure { rect, .. } => {
                let (a, b) = (
                    f(Point::new(rect.x, rect.y)),
                    f(Point::new(rect.right(), rect.bottom())),
                );
                *rect = RectF::from_points(a, b);
            }
            Kind::Text { at, .. } => *at = f(*at),
            Kind::Marker { center, .. } => *center = f(*center),
        }
    }

    /// Whether a click at `p` (image pixels) selects this annotation.
    /// `tolerance` widens thin strokes so they are easy to hit.
    pub fn hit_test(&self, p: Point, tolerance: f32) -> bool {
        let reach = self.style.width / 2.0 + tolerance;
        match &self.kind {
            Kind::Pen { points } | Kind::Highlighter { points } => {
                dist_to_polyline(p, points) <= reach
            }
            Kind::Line { from, to } | Kind::Arrow { from, to } => {
                dist_to_segment(p, *from, *to) <= reach
            }
            Kind::Rectangle { rect } => {
                if self.style.fill.is_some() {
                    return rect.inflate(reach).contains(p);
                }
                rect.inflate(reach).contains(p) && !rect.inflate(-reach).contains(p)
            }
            Kind::Ellipse { rect } => {
                let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
                let (rx, ry) = (rect.w / 2.0 + reach, rect.h / 2.0 + reach);
                let outer = ((p.x - cx) / rx).powi(2) + ((p.y - cy) / ry).powi(2) <= 1.0;
                if self.style.fill.is_some() {
                    return outer;
                }
                let (irx, iry) = (
                    (rect.w / 2.0 - reach).max(0.0),
                    (rect.h / 2.0 - reach).max(0.0),
                );
                let inner = irx > 0.0
                    && iry > 0.0
                    && ((p.x - cx) / irx).powi(2) + ((p.y - cy) / iry).powi(2) < 1.0;
                outer && !inner
            }
            Kind::Text { .. } | Kind::Marker { .. } | Kind::Obscure { .. } => {
                self.geometry_bounds().inflate(tolerance).contains(p)
            }
        }
    }

    /// `Err` describes the first value that would break rendering (NaN, infinity,
    /// negative sizes). Used when loading files, which may be hand-edited or damaged.
    pub fn validate(&self) -> Result<(), String> {
        let bad = |what: &str| Err(format!("annotation {}: {what}", self.id.0));
        let rect_ok = |r: &RectF| {
            [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite()) && r.w >= 0.0 && r.h >= 0.0
        };
        if !(self.style.width.is_finite() && self.style.width >= 0.0) {
            return bad("invalid stroke width");
        }
        let ok = match &self.kind {
            Kind::Pen { points } | Kind::Highlighter { points } => {
                points.iter().all(|p| p.is_finite())
            }
            Kind::Line { from, to } | Kind::Arrow { from, to } => {
                from.is_finite() && to.is_finite()
            }
            Kind::Rectangle { rect } | Kind::Ellipse { rect } | Kind::Obscure { rect, .. } => {
                rect_ok(rect)
            }
            Kind::Text { at, size, .. } => at.is_finite() && size.is_finite() && *size > 0.0,
            Kind::Marker { center, radius, .. } => {
                center.is_finite() && radius.is_finite() && *radius > 0.0
            }
        };
        if ok {
            Ok(())
        } else {
            bad("invalid coordinates")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ann(kind: Kind) -> Annotation {
        Annotation {
            id: AnnotationId(1),
            kind,
            style: Style::default(),
        }
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> RectF {
        RectF { x, y, w, h }
    }

    #[test]
    fn rect_from_points_normalises_any_corner_order() {
        let r = RectF::from_points(Point::new(10.0, 40.0), Point::new(4.0, 8.0));
        assert_eq!(r, rect(4.0, 8.0, 6.0, 32.0));
    }

    #[test]
    fn bounds_include_stroke_and_geometry_bounds_do_not() {
        let a = ann(Kind::Rectangle {
            rect: rect(10.0, 10.0, 20.0, 20.0),
        });
        assert_eq!(a.geometry_bounds(), rect(10.0, 10.0, 20.0, 20.0));
        assert_eq!(a.bounds(), rect(8.0, 8.0, 24.0, 24.0));
        let pen = ann(Kind::Pen {
            points: vec![Point::new(0.0, 0.0), Point::new(10.0, 5.0)],
        });
        assert_eq!(pen.geometry_bounds(), rect(0.0, 0.0, 10.0, 5.0));
    }

    #[test]
    fn translate_moves_every_point() {
        let mut a = ann(Kind::Line {
            from: Point::new(1.0, 2.0),
            to: Point::new(3.0, 4.0),
        });
        a.translate(10.0, -1.0);
        assert_eq!(
            a.kind,
            Kind::Line {
                from: Point::new(11.0, 1.0),
                to: Point::new(13.0, 3.0)
            }
        );
    }

    #[test]
    fn transform_to_maps_geometry_and_scales_text_and_markers() {
        let mut a = ann(Kind::Arrow {
            from: Point::new(0.0, 0.0),
            to: Point::new(10.0, 10.0),
        });
        a.transform_to(rect(0.0, 0.0, 10.0, 10.0), rect(100.0, 50.0, 40.0, 20.0));
        assert_eq!(
            a.kind,
            Kind::Arrow {
                from: Point::new(100.0, 50.0),
                to: Point::new(140.0, 70.0)
            }
        );

        let mut t = ann(Kind::Text {
            at: Point::new(0.0, 0.0),
            text: "Hi".into(),
            size: 20.0,
            bold: false,
        });
        let g = t.geometry_bounds();
        t.transform_to(g, RectF { h: g.h * 2.0, ..g });
        assert!(matches!(t.kind, Kind::Text { size, .. } if (size - 40.0).abs() < 1e-3));

        let mut m = ann(Kind::Marker {
            center: Point::new(10.0, 10.0),
            number: 3,
            radius: 10.0,
        });
        let g = m.geometry_bounds();
        m.transform_to(g, rect(g.x, g.y, g.w * 3.0, g.h * 1.5));
        assert!(
            matches!(m.kind, Kind::Marker { radius, number: 3, .. } if (radius - 15.0).abs() < 1e-3)
        );
    }

    #[test]
    fn transform_with_zero_size_source_only_translates() {
        let mut a = ann(Kind::Line {
            from: Point::new(5.0, 5.0),
            to: Point::new(5.0, 5.0),
        });
        a.transform_to(rect(5.0, 5.0, 0.0, 0.0), rect(7.0, 9.0, 50.0, 50.0));
        assert_eq!(
            a.kind,
            Kind::Line {
                from: Point::new(7.0, 9.0),
                to: Point::new(7.0, 9.0)
            }
        );
    }

    #[test]
    fn hit_test_lines_use_distance_with_tolerance() {
        let a = ann(Kind::Line {
            from: Point::new(0.0, 0.0),
            to: Point::new(100.0, 0.0),
        });
        assert!(a.hit_test(Point::new(50.0, 1.0), 0.0));
        assert!(!a.hit_test(Point::new(50.0, 10.0), 2.0));
        assert!(a.hit_test(Point::new(50.0, 10.0), 8.0));
        assert!(!a.hit_test(Point::new(150.0, 0.0), 2.0));
    }

    #[test]
    fn hit_test_unfilled_shapes_select_only_the_outline() {
        let r = ann(Kind::Rectangle {
            rect: rect(0.0, 0.0, 100.0, 100.0),
        });
        assert!(r.hit_test(Point::new(0.0, 50.0), 2.0));
        assert!(!r.hit_test(Point::new(50.0, 50.0), 2.0));
        let e = ann(Kind::Ellipse {
            rect: rect(0.0, 0.0, 100.0, 100.0),
        });
        assert!(e.hit_test(Point::new(0.0, 50.0), 2.0));
        assert!(!e.hit_test(Point::new(50.0, 50.0), 2.0));
    }

    #[test]
    fn hit_test_filled_shapes_select_the_interior() {
        let mut r = ann(Kind::Rectangle {
            rect: rect(0.0, 0.0, 100.0, 100.0),
        });
        r.style.fill = Some(Rgba::WHITE);
        assert!(r.hit_test(Point::new(50.0, 50.0), 0.0));
        let mut e = ann(Kind::Ellipse {
            rect: rect(0.0, 0.0, 100.0, 100.0),
        });
        e.style.fill = Some(Rgba::WHITE);
        assert!(e.hit_test(Point::new(50.0, 50.0), 0.0));
        assert!(
            !e.hit_test(Point::new(2.0, 2.0), 0.0),
            "corner is outside the ellipse"
        );
    }

    #[test]
    fn hit_test_single_point_pen_stroke() {
        let a = ann(Kind::Pen {
            points: vec![Point::new(10.0, 10.0)],
        });
        assert!(a.hit_test(Point::new(11.0, 10.0), 0.0));
        assert!(!a.hit_test(Point::new(30.0, 10.0), 0.0));
    }

    #[test]
    fn validate_rejects_non_finite_and_negative_values() {
        assert!(ann(Kind::Rectangle {
            rect: rect(0.0, 0.0, 5.0, 5.0)
        })
        .validate()
        .is_ok());
        assert!(ann(Kind::Rectangle {
            rect: rect(f32::NAN, 0.0, 5.0, 5.0)
        })
        .validate()
        .is_err());
        assert!(ann(Kind::Rectangle {
            rect: rect(0.0, 0.0, -5.0, 5.0)
        })
        .validate()
        .is_err());
        assert!(ann(Kind::Pen {
            points: vec![Point::new(f32::INFINITY, 0.0)]
        })
        .validate()
        .is_err());
        assert!(ann(Kind::Marker {
            center: Point::new(0.0, 0.0),
            number: 1,
            radius: 0.0
        })
        .validate()
        .is_err());
        let mut s = ann(Kind::Line {
            from: Point::default(),
            to: Point::default(),
        });
        s.style.width = f32::NAN;
        assert!(s.validate().is_err());
    }

    #[test]
    fn kinds_serialise_with_a_type_tag() {
        let a = ann(Kind::Obscure {
            rect: rect(1.0, 2.0, 3.0, 4.0),
            mode: ObscureMode::Blur,
            strength: 9,
        });
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"type\":\"Obscure\""), "{json}");
        assert_eq!(serde_json::from_str::<Annotation>(&json).unwrap(), a);
    }
}
