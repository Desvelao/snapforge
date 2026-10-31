//! Image resizing for composition: area averaging when shrinking (so thin
//! lines and text stay readable instead of aliasing), linear when enlarging.

use crate::Image;

/// Resizes `src` to `tw` x `th`. Colours are averaged weighted by alpha.
pub fn resample(src: &Image, tw: u32, th: u32) -> Image {
    let (sw, sh) = (src.width as usize, src.height as usize);
    let (tw, th) = (tw.max(1) as usize, th.max(1) as usize);
    if (sw, sh) == (tw, th) {
        return src.clone();
    }
    // Premultiplied float rows: horizontal pass first (sh rows of tw pixels), then vertical.
    let weights_x = axis_weights(sw, tw);
    let weights_y = axis_weights(sh, th);
    let mut mid = vec![0f32; tw * sh * 4];
    for y in 0..sh {
        let row = &src.rgba[y * sw * 4..(y + 1) * sw * 4];
        for (x, taps) in weights_x.iter().enumerate() {
            let mut acc = [0f32; 4];
            for &(i, w) in taps {
                let p = &row[i * 4..i * 4 + 4];
                let a = f32::from(p[3]) / 255.0;
                acc[0] += f32::from(p[0]) * a * w;
                acc[1] += f32::from(p[1]) * a * w;
                acc[2] += f32::from(p[2]) * a * w;
                acc[3] += a * w;
            }
            mid[(y * tw + x) * 4..(y * tw + x) * 4 + 4].copy_from_slice(&acc);
        }
    }
    let mut rgba = vec![0u8; tw * th * 4];
    for (y, taps) in weights_y.iter().enumerate() {
        for x in 0..tw {
            let mut acc = [0f32; 4];
            for &(j, w) in taps {
                let p = &mid[(j * tw + x) * 4..(j * tw + x) * 4 + 4];
                for c in 0..4 {
                    acc[c] += p[c] * w;
                }
            }
            let a = acc[3];
            let o = (y * tw + x) * 4;
            if a > 0.0 {
                for c in 0..3 {
                    rgba[o + c] = (acc[c] / a).round().clamp(0.0, 255.0) as u8;
                }
            }
            rgba[o + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Image {
        width: tw as u32,
        height: th as u32,
        rgba,
    }
}

/// For each target index, the source indices and weights (summing to 1).
fn axis_weights(src: usize, dst: usize) -> Vec<Vec<(usize, f32)>> {
    let ratio = src as f32 / dst as f32;
    (0..dst)
        .map(|t| {
            if dst >= src {
                // Enlarging: interpolate between the two nearest source pixels.
                let pos = ((t as f32 + 0.5) * ratio - 0.5).clamp(0.0, (src - 1) as f32);
                let i = pos.floor() as usize;
                let f = pos - i as f32;
                if f < 1e-6 || i + 1 >= src {
                    vec![(i, 1.0)]
                } else {
                    vec![(i, 1.0 - f), (i + 1, f)]
                }
            } else {
                // Shrinking: every source pixel the target pixel covers, weighted by overlap.
                let (a, b) = (t as f32 * ratio, (t as f32 + 1.0) * ratio);
                let first = a.floor() as usize;
                let last = (b.ceil() as usize).min(src);
                let mut taps: Vec<(usize, f32)> = (first..last)
                    .map(|i| {
                        (
                            i,
                            (b.min(i as f32 + 1.0) - a.max(i as f32)).max(0.0) / ratio,
                        )
                    })
                    .collect();
                let total: f32 = taps.iter().map(|(_, w)| w).sum();
                taps.iter_mut().for_each(|(_, w)| *w /= total);
                taps
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Image {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend(f(x, y));
            }
        }
        Image {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn same_size_is_a_copy() {
        let a = img(5, 4, |x, y| [x as u8, y as u8, 7, 255]);
        assert_eq!(resample(&a, 5, 4), a);
    }

    #[test]
    fn a_solid_colour_stays_solid_at_any_size() {
        let a = img(37, 23, |_, _| [10, 200, 30, 255]);
        for (w, h) in [(5, 3), (37, 11), (80, 60), (1, 1), (100, 7)] {
            let r = resample(&a, w, h);
            assert_eq!((r.width, r.height), (w, h));
            assert!(
                r.rgba.chunks_exact(4).all(|p| p == [10, 200, 30, 255]),
                "{w}x{h}"
            );
        }
    }

    #[test]
    fn shrinking_averages_instead_of_dropping_pixels() {
        // 1px black/white stripes shrunk 2x must come out mid grey, not black or white.
        let a = img(8, 8, |x, _| {
            if x % 2 == 0 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let r = resample(&a, 4, 4);
        assert!(
            r.rgba.chunks_exact(4).all(|p| (126..=129).contains(&p[0])),
            "{:?}",
            &r.rgba[..4]
        );
        // Uneven ratio: 3 -> 2 keeps the overall average.
        let line = img(3, 1, |x, _| [x as u8 * 90, 0, 0, 255]);
        let r = resample(&line, 2, 1);
        let avg_in: f32 = (0.0 + 90.0 + 180.0) / 3.0;
        let avg_out = f32::from(r.rgba[0] / 2) + f32::from(r.rgba[4] / 2);
        assert!((avg_in - avg_out).abs() < 3.0, "{avg_in} vs {avg_out}");
    }

    #[test]
    fn enlarging_interpolates_between_neighbours() {
        let a = img(2, 1, |x, _| {
            if x == 0 {
                [0, 0, 0, 255]
            } else {
                [200, 200, 200, 255]
            }
        });
        let r = resample(&a, 4, 1);
        let reds: Vec<u8> = r.rgba.chunks_exact(4).map(|p| p[0]).collect();
        assert!(reds.windows(2).all(|w| w[0] <= w[1]), "{reds:?}");
        assert_eq!((reds[0], reds[3]), (0, 200));
    }

    #[test]
    fn transparent_pixels_do_not_bleed_colour() {
        let a = img(2, 1, |x, _| {
            if x == 0 {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 0]
            }
        });
        let r = resample(&a, 1, 1);
        assert_eq!(
            &r.rgba[..3],
            &[255, 0, 0],
            "the invisible green must not tint the result"
        );
        assert!((126..=129).contains(&r.rgba[3]));
    }
}
