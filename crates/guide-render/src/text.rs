use std::sync::OnceLock;

use ab_glyph::{point, Font, FontRef, Glyph, PxScale, ScaleFont};
use guide_core::{Annotation, Kind, Point, RectF, Rgba};
use tiny_skia::{BlendMode, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

/// Ubuntu Light, the font egui already ships; see `assets/UFL.txt`.
static FONT_DATA: &[u8] = include_bytes!("../assets/Ubuntu-Light.ttf");

fn font() -> &'static FontRef<'static> {
    static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
    FONT.get_or_init(|| FontRef::try_from_slice(FONT_DATA).expect("embedded font is valid"))
}

/// Fonts to try, in this order, for characters Ubuntu Light does not have (CJK, Arabic, Hebrew,
/// Devanagari...). They are not embedded (a CJK font alone would exceed the size budget): the
/// first font file found with one of these name parts in the usual system font folders is used.
const FALLBACK_NAMES: &[&str] = &[
    "NotoSansCJK",
    "NotoSansJP",
    "NotoSansSC",
    "SourceHanSans",
    "DroidSansFallback",
    "wqy-",
    "NotoSansArabic",
    "NotoSansHebrew",
    "NotoSansDevanagari",
    "NotoSansThai",
    "NotoSans-Regular",
    "DejaVuSans.ttf",
    "FreeSans",
    "unifont",
];

fn font_folders() -> Vec<std::path::PathBuf> {
    let mut v = vec![
        "/usr/share/fonts".into(),
        "/usr/local/share/fonts".into(),
        "/run/host/fonts".into(),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        v.push(home.join(".local/share/fonts"));
        v.push(home.join(".fonts"));
    }
    v
}

/// Font files under `dir` (not following links, at most 6 folders deep).
fn collect_font_files(dir: &std::path::Path, depth: u8, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let Ok(kind) = e.file_type() else { continue };
        let path = e.path();
        if kind.is_dir() && depth < 6 {
            collect_font_files(&path, depth + 1, out);
        } else if kind.is_file() {
            let ext = path
                .extension()
                .map(|x| x.to_string_lossy().to_ascii_lowercase());
            if matches!(ext.as_deref(), Some("ttf" | "otf" | "ttc")) {
                out.push(path);
            }
        }
    }
}

/// The installed fonts to fall back to, most preferred first. Looked for once, and only when a
/// text really has a character the embedded font lacks.
fn fallbacks() -> &'static [FontRef<'static>] {
    static FALLBACKS: OnceLock<Vec<FontRef<'static>>> = OnceLock::new();
    FALLBACKS.get_or_init(|| {
        let mut files = Vec::new();
        for dir in font_folders() {
            collect_font_files(&dir, 0, &mut files);
        }
        files.sort();
        let mut out = Vec::new();
        for name in FALLBACK_NAMES {
            let Some(path) = files.iter().find(|p| {
                p.file_name()
                    .is_some_and(|f| f.to_string_lossy().contains(name))
            }) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            // Kept for the life of the program, like the embedded font.
            let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
            if let Ok(f) = FontRef::try_from_slice(bytes) {
                out.push(f);
            }
        }
        out
    })
}

/// Which font draws `ch`: 0 is the embedded one, `n` the n-th fallback. A character nobody has
/// is drawn by the embedded font (as its "missing" box).
fn font_index(ch: char) -> usize {
    if ch.is_control() || font().glyph_id(ch).0 != 0 {
        return 0;
    }
    fallbacks()
        .iter()
        .position(|f| f.glyph_id(ch).0 != 0)
        .map_or(0, |i| i + 1)
}

fn font_by_index(i: usize) -> &'static FontRef<'static> {
    if i == 0 {
        font()
    } else {
        &fallbacks()[i - 1]
    }
}

struct Layout {
    /// The font index (see [`font_index`]) of each glyph, and the glyph.
    glyphs: Vec<(usize, Glyph)>,
    width: f32,
    height: f32,
}

fn layout(text: &str, size: f32, origin: Point) -> Layout {
    let scale = PxScale::from(size.max(1.0));
    let scaled = font().as_scaled(scale);
    let line_height = scaled.height() + scaled.line_gap();
    let mut glyphs = Vec::new();
    let (mut width, mut lines) = (0.0f32, 0usize);
    for (row, line) in text.split('\n').enumerate() {
        lines += 1;
        let (mut x, mut prev) = (0.0f32, None);
        for ch in line.chars() {
            let which = font_index(ch);
            let f = font_by_index(which).as_scaled(scale);
            let id = f.glyph_id(ch);
            if let Some((p_which, p)) = prev {
                if p_which == which {
                    x += f.kern(p, id);
                }
            }
            glyphs.push((
                which,
                id.with_scale_and_position(
                    scale,
                    point(
                        origin.x + x,
                        origin.y + scaled.ascent() + row as f32 * line_height,
                    ),
                ),
            ));
            x += f.h_advance(id);
            prev = Some((which, id));
        }
        width = width.max(x);
    }
    Layout {
        glyphs,
        width,
        height: lines as f32 * line_height,
    }
}

/// Exact size in pixels of `text` as it will be rendered.
pub fn measure_text(text: &str, size: f32) -> (f32, f32) {
    let l = layout(text, size, Point::default());
    (l.width, l.height)
}

/// Composites `rgb` at `alpha` (0..1) onto one premultiplied pixel.
fn blend_pixel(pm: &mut Pixmap, x: i32, y: i32, rgb: [u8; 3], alpha: f32) {
    if x < 0 || y < 0 || x >= pm.width() as i32 || y >= pm.height() as i32 || alpha <= 0.0 {
        return;
    }
    let i = (y as usize * pm.width() as usize + x as usize) * 4;
    let d = &mut pm.data_mut()[i..i + 4];
    let a = alpha.min(1.0);
    for c in 0..3 {
        d[c] = (f32::from(d[c]) * (1.0 - a) + f32::from(rgb[c]) * a).round() as u8;
    }
    d[3] = (f32::from(d[3]) * (1.0 - a) + 255.0 * a).round() as u8;
}

fn draw_glyphs(pm: &mut Pixmap, l: &Layout, color: Rgba, dx: f32) {
    let alpha = f32::from(color.0[3]) / 255.0;
    for (which, g) in &l.glyphs {
        let mut g = g.clone();
        g.position.x += dx;
        if let Some(og) = font_by_index(*which).outline_glyph(g) {
            let b = og.px_bounds();
            og.draw(|gx, gy, cov| {
                blend_pixel(
                    pm,
                    b.min.x as i32 + gx as i32,
                    b.min.y as i32 + gy as i32,
                    [color.0[0], color.0[1], color.0[2]],
                    cov * alpha,
                );
            });
        }
    }
}

fn fill_rect(pm: &mut Pixmap, r: &RectF, color: Rgba) {
    if let Some(rect) = Rect::from_xywh(r.x, r.y, r.w, r.h) {
        let mut p = Paint::default();
        p.set_color_rgba8(color.0[0], color.0[1], color.0[2], color.0[3]);
        p.blend_mode = BlendMode::SourceOver;
        pm.fill_rect(rect, &p, Transform::identity(), None);
    }
}

/// Draws text and numbered markers.
pub fn draw(pm: &mut Pixmap, a: &Annotation) {
    match &a.kind {
        Kind::Text {
            at,
            text,
            size,
            bold,
        } => {
            let l = layout(text, *size, *at);
            if let Some(bg) = a.style.fill {
                let pad = (*size * 0.25).max(2.0);
                fill_rect(
                    pm,
                    &RectF {
                        x: at.x - pad,
                        y: at.y - pad,
                        w: l.width + 2.0 * pad,
                        h: l.height + 2.0 * pad,
                    },
                    bg,
                );
            }
            draw_glyphs(pm, &l, a.style.color, 0.0);
            if *bold {
                // The font has no bold face; a second pass a fraction of the size to the right thickens the strokes.
                draw_glyphs(pm, &l, a.style.color, (*size / 18.0).max(0.6));
            }
        }
        Kind::Marker {
            center,
            number,
            radius,
        } => {
            let ring = (radius / 8.0).max(1.5);
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            paint.set_color_rgba8(255, 255, 255, 255);
            if let Some(path) = PathBuilder::from_circle(center.x, center.y, radius + ring) {
                pm.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
            let c = a.style.color.0;
            paint.set_color_rgba8(c[0], c[1], c[2], 255);
            if let Some(path) = PathBuilder::from_circle(center.x, center.y, *radius) {
                pm.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
            let digits = number.to_string();
            let size = radius * if digits.len() > 2 { 0.85 } else { 1.2 };
            let (w, h) = measure_text(&digits, size);
            let l = layout(
                &digits,
                size,
                Point::new(center.x - w / 2.0, center.y - h / 2.0),
            );
            draw_glyphs(pm, &l, Rgba::WHITE, 0.0);
            draw_glyphs(pm, &l, Rgba::WHITE, (size / 18.0).max(0.6));
        }
        _ => {}
    }
}

/// Draws plain text (no annotation) with its top-left at `origin`.
pub(crate) fn draw_plain(
    pm: &mut Pixmap,
    text: &str,
    size: f32,
    bold: bool,
    origin: Point,
    color: Rgba,
) {
    let l = layout(text, size, origin);
    draw_glyphs(pm, &l, color, 0.0);
    if bold {
        draw_glyphs(pm, &l, color, (size / 18.0).max(0.6));
    }
}

/// Draws bold text centred on `center`.
pub(crate) fn draw_centered(pm: &mut Pixmap, text: &str, size: f32, center: Point, color: Rgba) {
    let (w, h) = measure_text(text, size);
    draw_plain(
        pm,
        text,
        size,
        true,
        Point::new(center.x - w / 2.0, center.y - h / 2.0),
        color,
    );
}

/// Height of one line of text at `size`.
pub fn line_height(size: f32) -> f32 {
    measure_text("Ag", size).1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters_the_embedded_font_has_never_leave_it() {
        for ch in "Hello, wörld! Привет Ωμέγα 123".chars() {
            assert_eq!(font_index(ch), 0, "{ch:?}");
        }
        assert_eq!(font_index('\n'), 0);
    }

    #[test]
    fn a_character_nobody_has_is_drawn_by_the_embedded_font() {
        assert_eq!(font_index('\u{10FFFD}'), 0);
    }

    /// Needs an installed font with Hebrew letters (DejaVu Sans is on nearly every system);
    /// without one there is nothing to fall back to and the test says so.
    #[test]
    fn text_in_another_script_uses_an_installed_font_when_there_is_one() {
        let which = font_index('א');
        if which == 0 {
            eprintln!("skipped: no installed font with Hebrew letters");
            return;
        }
        let (w, h) = measure_text("אבג", 24.0);
        assert!(w > 20.0 && h > 10.0, "the letters take room: {w}x{h}");
        let mut pm = Pixmap::new(80, 40).unwrap();
        let l = layout("אבג", 24.0, Point::new(4.0, 4.0));
        draw_glyphs(&mut pm, &l, Rgba([0, 0, 0, 255]), 0.0);
        let ink = pm.pixels().iter().filter(|p| p.alpha() > 0).count();
        assert!(ink > 100, "the letters are drawn, {ink} pixels");
        // Latin text next to it still comes from the embedded font.
        assert_eq!(font_index('a'), 0);
    }
}
