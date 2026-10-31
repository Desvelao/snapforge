use crate::Rect;

/// Smallest accepted selection edge, in image pixels; smaller drags are
/// treated as accidental clicks.
const MIN_EDGE: u32 = 3;

/// Converts a drag on a view into a pixel rectangle of the displayed image.
///
/// * `start`/`end`: drag points relative to the view's top-left (view units).
/// * `view`: size of the view the image is stretched over (view units).
/// * `image`: size of the displayed image in pixels.
/// * `origin`: where that image sits inside the full desktop capture, so the
///   result is in full-capture coordinates.
///
/// The drag may go in any direction and may leave the view; it is clamped.
/// Returns `None` for a degenerate view or a selection under [`MIN_EDGE`].
pub fn region_from_drag(
    start: (f32, f32),
    end: (f32, f32),
    view: (f32, f32),
    image: (u32, u32),
    origin: (u32, u32),
) -> Option<Rect> {
    if !(view.0 > 0.0 && view.1 > 0.0) || image.0 == 0 || image.1 == 0 {
        return None;
    }
    let to_px = |v: f32, view_len: f32, img_len: u32| {
        let t = (v / view_len).clamp(0.0, 1.0);
        (t * img_len as f32).round() as u32
    };
    let (x0, x1) = (
        to_px(start.0.min(end.0), view.0, image.0),
        to_px(start.0.max(end.0), view.0, image.0),
    );
    let (y0, y1) = (
        to_px(start.1.min(end.1), view.1, image.1),
        to_px(start.1.max(end.1), view.1, image.1),
    );
    let (w, h) = (x1 - x0, y1 - y0);
    if w < MIN_EDGE || h < MIN_EDGE {
        return None;
    }
    Some(Rect {
        x: origin.0 + x0,
        y: origin.1 + y0,
        w,
        h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: fn(u32, u32, u32, u32) -> Option<Rect> = |x, y, w, h| Some(Rect { x, y, w, h });

    #[test]
    fn one_to_one_view() {
        let r = region_from_drag(
            (10.0, 20.0),
            (110.0, 70.0),
            (800.0, 600.0),
            (800, 600),
            (0, 0),
        );
        assert_eq!(r, R(10, 20, 100, 50));
    }

    #[test]
    fn drag_direction_does_not_matter() {
        let a = region_from_drag(
            (110.0, 70.0),
            (10.0, 20.0),
            (800.0, 600.0),
            (800, 600),
            (0, 0),
        );
        assert_eq!(a, R(10, 20, 100, 50));
    }

    #[test]
    fn hidpi_scales_view_units_to_pixels() {
        // 2x: 400x300 points show an 800x600 image.
        let r = region_from_drag(
            (10.0, 20.0),
            (60.0, 45.0),
            (400.0, 300.0),
            (800, 600),
            (0, 0),
        );
        assert_eq!(r, R(20, 40, 100, 50));
    }

    #[test]
    fn monitor_origin_offsets_into_full_capture() {
        let r = region_from_drag(
            (0.0, 0.0),
            (100.0, 100.0),
            (1920.0, 1080.0),
            (1920, 1080),
            (1920, 0),
        );
        assert_eq!(r, R(1920, 0, 100, 100));
    }

    #[test]
    fn drag_outside_view_is_clamped() {
        let r = region_from_drag(
            (-50.0, -50.0),
            (900.0, 700.0),
            (800.0, 600.0),
            (800, 600),
            (0, 0),
        );
        assert_eq!(r, R(0, 0, 800, 600));
    }

    #[test]
    fn tiny_or_degenerate_selections_are_rejected() {
        assert_eq!(
            region_from_drag((5.0, 5.0), (6.0, 80.0), (800.0, 600.0), (800, 600), (0, 0)),
            None
        );
        assert_eq!(
            region_from_drag((0.0, 0.0), (9.0, 9.0), (0.0, 600.0), (800, 600), (0, 0)),
            None
        );
        assert_eq!(
            region_from_drag((0.0, 0.0), (9.0, 9.0), (800.0, 600.0), (0, 0), (0, 0)),
            None
        );
    }
}
