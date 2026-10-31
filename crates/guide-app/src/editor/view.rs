//! Mapping between image pixels and screen points, plus resize-handle geometry.
//! Pure maths, no drawing, so it is unit-tested directly.

use eframe::egui::{pos2, Pos2, Rect, Vec2};
use guide_core::{Point, RectF};

/// Canvas transform. `zoom` is screen *points* per image pixel (so 100% is
/// `1 / pixels_per_point`); `origin` is where image pixel (0, 0) is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub origin: Pos2,
    pub zoom: f32,
}

pub const MIN_ZOOM: f32 = 0.02;
pub const MAX_ZOOM: f32 = 32.0;

impl View {
    pub fn to_screen(self, p: Point) -> Pos2 {
        pos2(
            self.origin.x + p.x * self.zoom,
            self.origin.y + p.y * self.zoom,
        )
    }

    pub fn to_image(self, s: Pos2) -> Point {
        Point::new(
            (s.x - self.origin.x) / self.zoom,
            (s.y - self.origin.y) / self.zoom,
        )
    }

    pub fn screen_rect(&self, r: RectF) -> Rect {
        Rect::from_min_max(
            self.to_screen(Point::new(r.x, r.y)),
            self.to_screen(Point::new(r.right(), r.bottom())),
        )
    }

    /// Largest zoom that shows the whole image inside `canvas` (never above `max_zoom`), centred.
    pub fn fit(canvas: Rect, image: (u32, u32), max_zoom: f32) -> View {
        let (iw, ih) = (image.0.max(1) as f32, image.1.max(1) as f32);
        let zoom = (canvas.width() / iw)
            .min(canvas.height() / ih)
            .min(max_zoom)
            .clamp(MIN_ZOOM, MAX_ZOOM);
        Self::centred(canvas, image, zoom)
    }

    pub fn centred(canvas: Rect, image: (u32, u32), zoom: f32) -> View {
        let size = Vec2::new(image.0 as f32 * zoom, image.1 as f32 * zoom);
        View {
            origin: canvas.center() - size / 2.0,
            zoom,
        }
    }

    /// Multiplies the zoom by `factor` keeping the image point under `anchor` fixed.
    pub fn zoom_at(&mut self, anchor: Pos2, factor: f32) {
        let before = self.to_image(anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.origin = pos2(
            anchor.x - before.x * self.zoom,
            anchor.y - before.y * self.zoom,
        );
    }

    pub fn pan(&mut self, delta: Vec2) {
        self.origin += delta;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

impl Handle {
    pub const ALL: [Handle; 8] = [
        Handle::NW,
        Handle::N,
        Handle::NE,
        Handle::E,
        Handle::SE,
        Handle::S,
        Handle::SW,
        Handle::W,
    ];

    pub fn position(self, r: RectF) -> Point {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        match self {
            Handle::NW => Point::new(r.x, r.y),
            Handle::N => Point::new(cx, r.y),
            Handle::NE => Point::new(r.right(), r.y),
            Handle::E => Point::new(r.right(), cy),
            Handle::SE => Point::new(r.right(), r.bottom()),
            Handle::S => Point::new(cx, r.bottom()),
            Handle::SW => Point::new(r.x, r.bottom()),
            Handle::W => Point::new(r.x, cy),
        }
    }
}

/// The rectangle after dragging `handle` of `orig` to `to`. The opposite edge or
/// corner stays put; dragging past it flips the rectangle. Never smaller than 1x1.
pub fn resize_rect(orig: RectF, handle: Handle, to: Point) -> RectF {
    let (mut l, mut t, mut r, mut b) = (orig.x, orig.y, orig.right(), orig.bottom());
    match handle {
        Handle::NW => (l, t) = (to.x, to.y),
        Handle::N => t = to.y,
        Handle::NE => (r, t) = (to.x, to.y),
        Handle::E => r = to.x,
        Handle::SE => (r, b) = (to.x, to.y),
        Handle::S => b = to.y,
        Handle::SW => (l, b) = (to.x, to.y),
        Handle::W => l = to.x,
    }
    let rect = RectF::from_points(Point::new(l, t), Point::new(r, b));
    RectF {
        w: rect.w.max(1.0),
        h: rect.h.max(1.0),
        ..rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Rect {
        Rect::from_min_max(pos2(100.0, 50.0), pos2(900.0, 650.0))
    }

    #[test]
    fn screen_and_image_coordinates_round_trip() {
        let v = View {
            origin: pos2(30.0, 40.0),
            zoom: 2.5,
        };
        let p = Point::new(123.0, 77.0);
        let back = v.to_image(v.to_screen(p));
        assert!((back.x - p.x).abs() < 1e-4 && (back.y - p.y).abs() < 1e-4);
        assert_eq!(v.to_screen(Point::new(0.0, 0.0)), pos2(30.0, 40.0));
    }

    #[test]
    fn fit_centres_and_never_exceeds_max_zoom() {
        let v = View::fit(canvas(), (1600, 600), 1.0);
        assert_eq!(v.zoom, 0.5, "800 / 1600 limits the zoom");
        let size = Vec2::new(1600.0 * 0.5, 600.0 * 0.5);
        assert_eq!(v.origin, canvas().center() - size / 2.0);
        let small = View::fit(canvas(), (100, 100), 1.0);
        assert_eq!(small.zoom, 1.0, "small images are not enlarged");
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer_fixed() {
        let mut v = View::fit(canvas(), (1000, 800), 1.0);
        let anchor = pos2(400.0, 300.0);
        let under = v.to_image(anchor);
        for f in [1.5, 1.5, 0.3, 4.0] {
            v.zoom_at(anchor, f);
            let now = v.to_image(anchor);
            assert!(
                (now.x - under.x).abs() < 1e-2 && (now.y - under.y).abs() < 1e-2,
                "{now:?} vs {under:?}"
            );
        }
    }

    #[test]
    fn zoom_is_clamped() {
        let mut v = View {
            origin: pos2(0.0, 0.0),
            zoom: 1.0,
        };
        v.zoom_at(pos2(0.0, 0.0), 1e9);
        assert_eq!(v.zoom, MAX_ZOOM);
        v.zoom_at(pos2(0.0, 0.0), 1e-9);
        assert_eq!(v.zoom, MIN_ZOOM);
    }

    #[test]
    fn pan_moves_the_origin() {
        let mut v = View {
            origin: pos2(10.0, 10.0),
            zoom: 1.0,
        };
        v.pan(Vec2::new(5.0, -3.0));
        assert_eq!(v.origin, pos2(15.0, 7.0));
    }

    fn rect() -> RectF {
        RectF {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 60.0,
        }
    }

    #[test]
    fn handles_sit_on_the_rectangle_edges() {
        assert_eq!(Handle::NW.position(rect()), Point::new(10.0, 20.0));
        assert_eq!(Handle::E.position(rect()), Point::new(110.0, 50.0));
        assert_eq!(Handle::S.position(rect()), Point::new(60.0, 80.0));
    }

    #[test]
    fn dragging_a_corner_keeps_the_opposite_corner() {
        let r = resize_rect(rect(), Handle::SE, Point::new(200.0, 150.0));
        assert_eq!(
            r,
            RectF {
                x: 10.0,
                y: 20.0,
                w: 190.0,
                h: 130.0
            }
        );
        let r = resize_rect(rect(), Handle::NW, Point::new(0.0, 0.0));
        assert_eq!(
            r,
            RectF {
                x: 0.0,
                y: 0.0,
                w: 110.0,
                h: 80.0
            }
        );
    }

    #[test]
    fn dragging_an_edge_changes_one_axis_only() {
        let r = resize_rect(rect(), Handle::E, Point::new(60.0, 999.0));
        assert_eq!(
            r,
            RectF {
                x: 10.0,
                y: 20.0,
                w: 50.0,
                h: 60.0
            }
        );
        let r = resize_rect(rect(), Handle::N, Point::new(999.0, 40.0));
        assert_eq!(
            r,
            RectF {
                x: 10.0,
                y: 40.0,
                w: 100.0,
                h: 40.0
            }
        );
    }

    #[test]
    fn dragging_past_the_opposite_side_flips_and_size_stays_positive() {
        let r = resize_rect(rect(), Handle::E, Point::new(-30.0, 0.0));
        assert_eq!(
            r,
            RectF {
                x: -30.0,
                y: 20.0,
                w: 40.0,
                h: 60.0
            }
        );
        let r = resize_rect(rect(), Handle::E, Point::new(10.0, 0.0));
        assert_eq!(r.w, 1.0);
    }
}
