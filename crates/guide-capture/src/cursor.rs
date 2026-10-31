use crate::Screenshot;

/// Pointer image as X11 XFixes reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorImage {
    /// Pointer position (hotspot) in desktop pixels.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Hotspot offset inside the image.
    pub xhot: u32,
    pub yhot: u32,
    /// `width * height` pixels, `0xAARRGGBB` with premultiplied alpha.
    pub argb: Vec<u32>,
}

/// Blends `cursor` into `shot`, clipping at the edges. Premultiplied alpha
/// means `out = src + dst * (1 - alpha)`. A malformed image (pixel count not
/// matching its size) is ignored.
pub fn composite_cursor(shot: &mut Screenshot, cursor: &CursorImage) {
    if cursor.argb.len() != cursor.width as usize * cursor.height as usize {
        return;
    }
    let left = i64::from(cursor.x) - i64::from(cursor.xhot);
    let top = i64::from(cursor.y) - i64::from(cursor.yhot);
    for cy in 0..cursor.height {
        for cx in 0..cursor.width {
            let (x, y) = (left + i64::from(cx), top + i64::from(cy));
            if x < 0 || y < 0 || x >= i64::from(shot.width) || y >= i64::from(shot.height) {
                continue;
            }
            let px = cursor.argb[(cy * cursor.width + cx) as usize];
            let a = px >> 24;
            if a == 0 {
                continue;
            }
            let src = [(px >> 16) & 255, (px >> 8) & 255, px & 255];
            let i = (y as usize * shot.width as usize + x as usize) * 4;
            for (c, s) in src.iter().enumerate() {
                let dst = u32::from(shot.rgba[i + c]);
                shot.rgba[i + c] = (s + (dst * (255 - a) + 127) / 255).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Screenshot {
        Screenshot {
            width: 4,
            height: 4,
            rgba: [100, 100, 100, 255].repeat(16),
        }
    }

    fn px(shot: &Screenshot, x: usize, y: usize) -> [u8; 4] {
        let i = (y * 4 + x) * 4;
        [
            shot.rgba[i],
            shot.rgba[i + 1],
            shot.rgba[i + 2],
            shot.rgba[i + 3],
        ]
    }

    fn cursor(x: i32, y: i32, argb: Vec<u32>, w: u32, h: u32) -> CursorImage {
        CursorImage {
            x,
            y,
            width: w,
            height: h,
            xhot: 0,
            yhot: 0,
            argb,
        }
    }

    #[test]
    fn opaque_pixel_replaces_background() {
        let mut s = canvas();
        composite_cursor(&mut s, &cursor(1, 2, vec![0xFF_FF_00_00], 1, 1));
        assert_eq!(px(&s, 1, 2), [255, 0, 0, 255]);
        assert_eq!(px(&s, 0, 0), [100, 100, 100, 255]);
    }

    #[test]
    fn transparent_pixel_leaves_background() {
        let mut s = canvas();
        composite_cursor(&mut s, &cursor(1, 1, vec![0], 1, 1));
        assert_eq!(px(&s, 1, 1), [100, 100, 100, 255]);
    }

    #[test]
    fn half_alpha_is_premultiplied_over_background() {
        // 50% white, premultiplied: rgb = 128 each, alpha = 128.
        let mut s = canvas();
        composite_cursor(&mut s, &cursor(0, 0, vec![0x80_80_80_80], 1, 1));
        // 128 + 100 * 127 / 255 = 128 + 50 = 178
        assert_eq!(px(&s, 0, 0), [178, 178, 178, 255]);
    }

    #[test]
    fn hotspot_offsets_the_image() {
        let mut s = canvas();
        let mut c = cursor(2, 2, vec![0xFF_00_FF_00], 1, 1);
        c.xhot = 1;
        c.yhot = 1;
        composite_cursor(&mut s, &c);
        assert_eq!(px(&s, 1, 1), [0, 255, 0, 255]);
        assert_eq!(px(&s, 2, 2), [100, 100, 100, 255]);
    }

    #[test]
    fn clips_at_edges_without_panicking() {
        let mut s = canvas();
        let c = cursor(3, 3, vec![0xFF_FF_FF_FF; 4], 2, 2);
        composite_cursor(&mut s, &c);
        assert_eq!(px(&s, 3, 3), [255, 255, 255, 255]);
        let mut s2 = canvas();
        let mut off = cursor(0, 0, vec![0xFF_FF_FF_FF; 4], 2, 2);
        off.xhot = 5; // image entirely left/above the screen
        off.yhot = 5;
        composite_cursor(&mut s2, &off);
        assert_eq!(s2, canvas());
    }

    #[test]
    fn malformed_image_is_ignored() {
        let mut s = canvas();
        composite_cursor(&mut s, &cursor(0, 0, vec![0xFF_FF_FF_FF], 2, 2));
        assert_eq!(s, canvas());
    }
}
