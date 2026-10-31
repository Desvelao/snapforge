//! Pixelate and blur. These rewrite the actual pixels of the image, so the
//! redacted content cannot be recovered from the exported file.

use guide_core::{ObscureMode, RectF};
use tiny_skia::Pixmap;

/// Integer pixel region clipped to the image, or `None` if it is empty.
struct Region {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

fn region(rect: RectF, width: u32, height: u32) -> Option<Region> {
    let x0 = rect.x.floor().max(0.0) as usize;
    let y0 = rect.y.floor().max(0.0) as usize;
    let x1 = (rect.right().ceil().max(0.0) as usize).min(width as usize);
    let y1 = (rect.bottom().ceil().max(0.0) as usize).min(height as usize);
    // `then`, not `then_some`: the subtraction would underflow for a region outside the image.
    (x1 > x0 && y1 > y0).then(|| Region {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}

pub fn apply(pm: &mut Pixmap, rect: RectF, mode: ObscureMode, strength: u32) {
    let Some(r) = region(rect, pm.width(), pm.height()) else {
        return;
    };
    let stride = pm.width() as usize;
    match mode {
        ObscureMode::Pixelate => pixelate(pm.data_mut(), stride, &r, strength.max(2) as usize),
        ObscureMode::Blur => blur(pm.data_mut(), stride, &r, strength.clamp(1, 200) as usize),
    }
}

/// Replaces every `block` x `block` cell (anchored at the region's top-left) with its average colour.
fn pixelate(data: &mut [u8], stride: usize, r: &Region, block: usize) {
    for by in (0..r.h).step_by(block) {
        for bx in (0..r.w).step_by(block) {
            let (bw, bh) = (block.min(r.w - bx), block.min(r.h - by));
            let mut sum = [0u64; 4];
            for y in 0..bh {
                for x in 0..bw {
                    let i = ((r.y + by + y) * stride + r.x + bx + x) * 4;
                    for c in 0..4 {
                        sum[c] += u64::from(data[i + c]);
                    }
                }
            }
            let n = (bw * bh) as u64;
            let avg: [u8; 4] = std::array::from_fn(|c| ((sum[c] + n / 2) / n) as u8);
            for y in 0..bh {
                for x in 0..bw {
                    let i = ((r.y + by + y) * stride + r.x + bx + x) * 4;
                    data[i..i + 4].copy_from_slice(&avg);
                }
            }
        }
    }
}

/// Three box-blur passes (a close match for a Gaussian) confined to the region.
fn blur(data: &mut [u8], stride: usize, r: &Region, radius: usize) {
    let mut buf: Vec<u8> = Vec::with_capacity(r.w * r.h * 4);
    for y in 0..r.h {
        let i = ((r.y + y) * stride + r.x) * 4;
        buf.extend_from_slice(&data[i..i + r.w * 4]);
    }
    let mut tmp = vec![0u8; buf.len()];
    for _ in 0..3 {
        box_pass(&buf, &mut tmp, r.w, r.h, radius, true);
        box_pass(&tmp, &mut buf, r.w, r.h, radius, false);
    }
    for y in 0..r.h {
        let i = ((r.y + y) * stride + r.x) * 4;
        data[i..i + r.w * 4].copy_from_slice(&buf[y * r.w * 4..(y + 1) * r.w * 4]);
    }
}

/// One horizontal or vertical box blur using running sums; the window shrinks at the edges.
fn box_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, radius: usize, horizontal: bool) {
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let at = |line: usize, pos: usize| {
        if horizontal {
            (line * w + pos) * 4
        } else {
            (pos * w + line) * 4
        }
    };
    for line in 0..lines {
        let mut sum = [0u32; 4];
        let (mut lo, mut hi) = (0usize, 0usize); // window is [lo, hi)
        for pos in 0..len {
            let want_hi = (pos + radius + 1).min(len);
            let want_lo = pos.saturating_sub(radius);
            while hi < want_hi {
                for c in 0..4 {
                    sum[c] += u32::from(src[at(line, hi) + c]);
                }
                hi += 1;
            }
            while lo < want_lo {
                for c in 0..4 {
                    sum[c] -= u32::from(src[at(line, lo) + c]);
                }
                lo += 1;
            }
            let n = (hi - lo) as u32;
            let o = at(line, pos);
            for c in 0..4 {
                dst[o + c] = ((sum[c] + n / 2) / n) as u8;
            }
        }
    }
}
