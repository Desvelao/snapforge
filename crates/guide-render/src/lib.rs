//! Annotation rendering (and, later, guide composition and export), with no UI.
//!
//! [`render`] draws a [`Document`] onto a copy of the original image and
//! applies the crop. It never modifies the original.

pub mod compose;
mod obscure;
pub mod resample;
mod shapes;
mod text;

use guide_core::{AnnotationId, Document, Kind, PixelRect};
use tiny_skia::Pixmap;

pub use text::{line_height, measure_text};

/// Images above this many pixels are refused instead of risking an
/// out-of-memory abort (about 480 MB per copy at 4 bytes per pixel).
pub const MAX_PIXELS: u64 = 120_000_000;

/// Straight (non-premultiplied) RGBA8, row-major, no padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RenderError {
    #[error("image of {0}x{1} pixels is too large to render (limit {MAX_PIXELS} pixels); crop it or export smaller")]
    TooLarge(u32, u32),
    #[error("the image data does not match its size")]
    InvalidImage,
    #[error("invalid annotations: {0}")]
    InvalidDocument(String),
    #[error("could not allocate the drawing surface")]
    Alloc,
}

#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// Leave this annotation out (the editor draws it live while it is dragged).
    pub exclude: Option<AnnotationId>,
    /// Cut the result to the document's crop. Off for the editor canvas,
    /// which shows the whole image with the crop outlined.
    pub apply_crop: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            exclude: None,
            apply_crop: true,
        }
    }
}

/// Renders the finished image: annotations drawn in order, then cropped.
pub fn render(original: &Image, doc: &Document) -> Result<Image, RenderError> {
    render_with(original, doc, &RenderOptions::default())
}

pub fn render_with(
    original: &Image,
    doc: &Document,
    opts: &RenderOptions,
) -> Result<Image, RenderError> {
    let (w, h) = (original.width, original.height);
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(RenderError::TooLarge(w, h));
    }
    if w == 0 || h == 0 || original.rgba.len() != w as usize * h as usize * 4 {
        return Err(RenderError::InvalidImage);
    }
    doc.validate((w, h)).map_err(RenderError::InvalidDocument)?;

    let mut pixmap = Pixmap::new(w, h).ok_or(RenderError::Alloc)?;
    premultiply_into(&original.rgba, pixmap.data_mut());

    for a in &doc.annotations {
        if Some(a.id) == opts.exclude {
            continue;
        }
        match &a.kind {
            Kind::Obscure {
                rect,
                mode,
                strength,
            } => obscure::apply(&mut pixmap, *rect, *mode, *strength),
            Kind::Text { .. } | Kind::Marker { .. } => text::draw(&mut pixmap, a),
            _ => shapes::draw(&mut pixmap, a),
        }
    }

    let crop = if opts.apply_crop { doc.crop } else { None };
    Ok(finish(&pixmap, crop))
}

pub(crate) fn premultiply_into(src: &[u8], dst: &mut [u8]) {
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        let a = u32::from(s[3]);
        for c in 0..3 {
            d[c] = ((u32::from(s[c]) * a + 127) / 255) as u8;
        }
        d[3] = s[3];
    }
}

pub(crate) fn finish(pixmap: &Pixmap, crop: Option<PixelRect>) -> Image {
    let full = PixelRect {
        x: 0,
        y: 0,
        w: pixmap.width(),
        h: pixmap.height(),
    };
    let r = crop.unwrap_or(full);
    let mut rgba = Vec::with_capacity(r.w as usize * r.h as usize * 4);
    let stride = pixmap.width() as usize * 4;
    for row in r.y..r.y + r.h {
        let start = row as usize * stride + r.x as usize * 4;
        for p in pixmap.data()[start..start + r.w as usize * 4].chunks_exact(4) {
            let a = u32::from(p[3]);
            if a == 0 || a == 255 {
                rgba.extend_from_slice(p);
            } else {
                rgba.extend((0..3).map(|c| ((u32::from(p[c]) * 255 + a / 2) / a).min(255) as u8));
                rgba.push(p[3]);
            }
        }
    }
    Image {
        width: r.w,
        height: r.h,
        rgba,
    }
}

#[cfg(test)]
mod tests;
