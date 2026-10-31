use crate::Rect;

/// Rectangle in desktop coordinates; may extend past the screen edges or
/// have a negative origin (windows can be partly off-screen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// A top-level window as seen by the window manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    /// Outer rectangle including the window-manager frame.
    pub rect: IRect,
}

/// Outer rectangle from the client area position/size and the frame extents
/// `[left, right, top, bottom]` (`_NET_FRAME_EXTENTS`).
pub fn frame_rect(pos: (i32, i32), size: (u32, u32), extents: [u32; 4]) -> IRect {
    let [l, r, t, b] = extents;
    IRect {
        x: pos.0.saturating_sub_unsigned(l),
        y: pos.1.saturating_sub_unsigned(t),
        w: size.0.saturating_add(l).saturating_add(r),
        h: size.1.saturating_add(t).saturating_add(b),
    }
}

/// The part of `r` that lies inside a `bounds`-sized screen, or `None` when
/// nothing is visible.
pub fn visible_rect(r: IRect, bounds: (u32, u32)) -> Option<Rect> {
    let x0 = i64::from(r.x).max(0);
    let y0 = i64::from(r.y).max(0);
    let x1 = (i64::from(r.x) + i64::from(r.w)).min(i64::from(bounds.0));
    let y1 = (i64::from(r.y) + i64::from(r.h)).min(i64::from(bounds.1));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(Rect {
        x: x0 as u32,
        y: y0 as u32,
        w: (x1 - x0) as u32,
        h: (y1 - y0) as u32,
    })
}

/// The window drawn on top at `point`. `windows` is in stacking order,
/// bottom first, as `_NET_CLIENT_LIST_STACKING` reports it.
pub fn topmost_at(
    windows: &[WindowInfo],
    point: (i32, i32),
    bounds: (u32, u32),
) -> Option<&WindowInfo> {
    windows.iter().rev().find(|w| {
        visible_rect(w.rect, bounds).is_some_and(|v| {
            point.0 >= v.x as i32
                && point.1 >= v.y as i32
                && point.0 < (v.x + v.w) as i32
                && point.1 < (v.y + v.h) as i32
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: u32, x: i32, y: i32, w: u32, h: u32) -> WindowInfo {
        WindowInfo {
            id,
            title: format!("w{id}"),
            rect: IRect { x, y, w, h },
        }
    }

    #[test]
    fn frame_adds_decorations_around_client_area() {
        let r = frame_rect((100, 130), (400, 300), [4, 4, 30, 4]);
        assert_eq!(
            r,
            IRect {
                x: 96,
                y: 100,
                w: 408,
                h: 334
            }
        );
    }

    #[test]
    fn frame_without_extents_is_the_client_rect() {
        assert_eq!(
            frame_rect((5, 6), (7, 8), [0; 4]),
            IRect {
                x: 5,
                y: 6,
                w: 7,
                h: 8
            }
        );
    }

    #[test]
    fn visible_rect_clips_to_screen() {
        let r = IRect {
            x: -20,
            y: 10,
            w: 100,
            h: 2000,
        };
        assert_eq!(
            visible_rect(r, (1280, 800)),
            Some(Rect {
                x: 0,
                y: 10,
                w: 80,
                h: 790
            })
        );
    }

    #[test]
    fn fully_offscreen_or_empty_is_not_visible() {
        assert_eq!(
            visible_rect(
                IRect {
                    x: 1280,
                    y: 0,
                    w: 50,
                    h: 50
                },
                (1280, 800)
            ),
            None
        );
        assert_eq!(
            visible_rect(
                IRect {
                    x: -60,
                    y: 0,
                    w: 50,
                    h: 50
                },
                (1280, 800)
            ),
            None
        );
        assert_eq!(
            visible_rect(
                IRect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 50
                },
                (1280, 800)
            ),
            None
        );
    }

    #[test]
    fn topmost_prefers_later_in_stacking_order() {
        let ws = [win(1, 0, 0, 500, 500), win(2, 100, 100, 200, 200)];
        assert_eq!(
            topmost_at(&ws, (150, 150), (1280, 800)).map(|w| w.id),
            Some(2)
        );
        assert_eq!(
            topmost_at(&ws, (10, 10), (1280, 800)).map(|w| w.id),
            Some(1)
        );
        assert_eq!(topmost_at(&ws, (900, 700), (1280, 800)), None);
    }

    #[test]
    fn edges_are_half_open() {
        let ws = [win(1, 10, 10, 10, 10)];
        assert!(topmost_at(&ws, (10, 10), (100, 100)).is_some());
        assert!(topmost_at(&ws, (19, 19), (100, 100)).is_some());
        assert!(topmost_at(&ws, (20, 20), (100, 100)).is_none());
    }
}
