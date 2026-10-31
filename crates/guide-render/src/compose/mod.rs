//! Guide composition: lays the included steps out into one image, and draws it.
//!
//! [`compute_layout`] works out every position, size and line of wrapped text in
//! output pixels, and the final image size, *before* anything is drawn. Both the
//! preview in the window and the exported image are produced from that same
//! [`ComposedLayout`] with [`render_layout`], so they cannot disagree about where
//! things go; the preview just draws it at a smaller `scale`.

use std::collections::HashMap;

use guide_core::{
    DescriptionPlacement, GuideSettings, ImageAlign, ImageScaling, Item, ItemId,
    Layout as LayoutKind, PixelRect, Point, Project, RectF, Rgba, StepLayout,
};
use tiny_skia::{
    BlendMode, FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Transform,
};

use crate::{resample::resample, text, Image, MAX_PIXELS};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ComposeError {
    #[error(
        "the combined image would be {0}x{1} pixels, more than the {MAX_PIXELS} pixel limit; \
         export the steps as separate images, use a narrower content width, or fewer steps"
    )]
    TooLarge(u32, u32),
    #[error("could not allocate the drawing surface")]
    Alloc,
    #[error("drawing was stopped")]
    Cancelled,
}

/// Wrapped text and where to draw it (top-left of the first line).
#[derive(Clone, Debug, PartialEq)]
pub struct TextItem {
    pub lines: Vec<String>,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub bold: bool,
    pub color: Rgba,
}

/// The numbered circle in front of a step title.
#[derive(Clone, Debug, PartialEq)]
pub struct BadgeItem {
    pub cx: f32,
    pub cy: f32,
    pub radius: f32,
    pub number: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageItem {
    pub step: ItemId,
    pub rect: RectF,
}

/// The whole area of one step, for hit-testing and highlighting in the preview.
#[derive(Clone, Debug, PartialEq)]
pub struct StepBox {
    pub step: ItemId,
    pub number: u32,
    pub rect: RectF,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComposedLayout {
    pub width: u32,
    pub height: u32,
    pub background: Rgba,
    pub texts: Vec<TextItem>,
    pub badges: Vec<BadgeItem>,
    pub images: Vec<ImageItem>,
    pub boxes: Vec<StepBox>,
    /// Text drawn on the grey placeholder of a screenshot that is missing; the caller may translate it.
    pub missing_label: String,
}

/// What [`ComposedLayout::missing_label`] says until the caller sets its own.
pub const DEFAULT_MISSING_LABEL: &str = "Screenshot missing";

const ACCENT: Rgba = Rgba([40, 110, 200, 255]);
/// A screenshot whose size is unknown (missing file) gets a 16:9 placeholder.
const PLACEHOLDER_RATIO: f32 = 9.0 / 16.0;

/// Greedy word wrap so that every line is at most `max_width` pixels wide at `size`.
/// Paragraph breaks (`\n`) are kept, over-long words are broken, nothing is dropped.
pub fn wrap_text(text: &str, size: f32, max_width: f32) -> Vec<String> {
    let width = |s: &str| text::measure_text(s, size).0;
    let mut out: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if width(&candidate) <= max_width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            if width(word) <= max_width {
                line = word.to_string();
            } else {
                let mut chunk = String::new();
                for ch in word.chars() {
                    let next = format!("{chunk}{ch}");
                    if chunk.is_empty() || width(&next) <= max_width {
                        chunk = next;
                    } else {
                        out.push(std::mem::take(&mut chunk));
                        chunk = ch.to_string();
                    }
                }
                line = chunk;
            }
        }
        out.push(line);
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

struct Builder<'a> {
    layout: ComposedLayout,
    settings: &'a guide_core::CompositionSettings,
    font: f32,
    /// Lowest point anything has been placed at so far.
    bottom: f32,
}

impl Builder<'_> {
    /// Adds wrapped text; returns its height (0 if there was nothing to draw).
    fn text(&mut self, text: &str, x: f32, y: f32, width: f32, size: f32, bold: bool) -> f32 {
        // Bold is drawn twice, a little apart, so it needs a little more room.
        let slack = if bold { size / 18.0 } else { 0.0 };
        let lines = wrap_text(text, size, (width - slack).max(1.0));
        if lines.is_empty() {
            return 0.0;
        }
        let height = lines.len() as f32 * text::line_height(size);
        self.layout.texts.push(TextItem {
            lines,
            x,
            y,
            size,
            bold,
            color: self.settings.text_color,
        });
        self.bottom = self.bottom.max(y + height);
        height
    }
}

/// Where every item goes. `sizes` gives each step's final screenshot size in pixels
/// (after cropping); a step without an entry gets a placeholder.
pub fn compute_layout(
    project: &Project,
    sizes: &HashMap<ItemId, (u32, u32)>,
) -> Result<ComposedLayout, ComposeError> {
    compute_layout_from(project, sizes, 1)
}

/// Like [`compute_layout`], but the first included step is numbered `first_number`. Used when a
/// single step of a guide is drawn on its own and must keep the number it has in the guide.
pub fn compute_layout_from(
    project: &Project,
    sizes: &HashMap<ItemId, (u32, u32)>,
    first_number: u32,
) -> Result<ComposedLayout, ComposeError> {
    // A plain project has no guide settings; it is composed with the defaults.
    let default_guide;
    let guide = match project.guide() {
        Some(g) => g,
        None => {
            default_guide = GuideSettings::default();
            &default_guide
        }
    };
    let c = &guide.composition;
    let margin = c.margin.min(2_000) as f32;
    let gap = c.spacing.min(2_000) as f32;
    let font = c.font_size.clamp(8.0, 120.0);
    let content_w = c.max_width.clamp(200, 10_000) as f32;
    let canvas_w = (content_w + 2.0 * margin).ceil();

    let mut b = Builder {
        layout: ComposedLayout {
            width: canvas_w as u32,
            height: 0,
            background: c.background,
            texts: Vec::new(),
            badges: Vec::new(),
            images: Vec::new(),
            boxes: Vec::new(),
            missing_label: DEFAULT_MISSING_LABEL.to_string(),
        },
        settings: c,
        font,
        bottom: margin,
    };
    let mut y = margin;

    if c.show_guide_title && !project.name.trim().is_empty() {
        y += b.text(&project.name, margin, y, content_w, font * 1.8, true) + gap * 0.5;
    }
    if c.show_introduction && !guide.introduction.trim().is_empty() {
        y += b.text(&guide.introduction, margin, y, content_w, font, false) + gap;
    }

    let columns = match c.layout {
        LayoutKind::Vertical => 1,
        LayoutKind::Grid { columns } => columns.clamp(1, 8) as usize,
    };
    let cell_w = ((content_w - (columns as f32 - 1.0) * gap) / columns as f32)
        .floor()
        .max(50.0);
    let included: Vec<&Item> = project.included_items().collect();
    let mut number = first_number.saturating_sub(1);
    for row in included.chunks(columns) {
        let mut row_h = 0.0f32;
        for (col, step) in row.iter().enumerate() {
            number += 1;
            let x = margin + col as f32 * (cell_w + gap);
            row_h = row_h.max(layout_step(
                &mut b,
                step,
                project.step_layout(step.id),
                number,
                (x, y),
                cell_w,
                sizes,
            ));
        }
        y += row_h + gap;
    }

    if c.show_final_notes && !guide.final_notes.trim().is_empty() {
        y += b.text(&guide.final_notes, margin, y, content_w, font, false);
    }
    let height = (b.bottom.max(y - gap).max(margin) + margin).ceil();
    if u64::from(canvas_w as u32) * height as u64 > MAX_PIXELS || height > f32::from(u16::MAX) * 4.0
    {
        return Err(ComposeError::TooLarge(
            canvas_w as u32,
            height.min(u32::MAX as f32) as u32,
        ));
    }
    b.layout.height = height as u32;
    Ok(b.layout)
}

/// Lays out one step in a slot `w` wide whose top-left is `(x, y)`; returns its height.
fn layout_step(
    b: &mut Builder,
    step: &Item,
    overrides: Option<&StepLayout>,
    number: u32,
    (x, y): (f32, f32),
    w: f32,
    sizes: &HashMap<ItemId, (u32, u32)>,
) -> f32 {
    let c = b.settings;
    let font = b.font;
    let small_gap = font * 0.5;
    let mut cy = y;

    // Number badge and title.
    let title_size = font * 1.25;
    let badge_d = title_size * 1.5;
    let has_badge = c.show_numbers;
    let has_title = c.show_titles && !step.title.trim().is_empty();
    if has_badge || has_title {
        let indent = if has_badge { badge_d + small_gap } else { 0.0 };
        let lines = if has_title {
            wrap_text(
                &step.title,
                title_size,
                (w - indent - title_size / 18.0).max(1.0),
            )
        } else {
            Vec::new()
        };
        let text_h = lines.len() as f32 * text::line_height(title_size);
        let row_h = text_h.max(if has_badge { badge_d } else { 0.0 });
        if has_badge {
            b.layout.badges.push(BadgeItem {
                cx: x + badge_d / 2.0,
                cy: cy + badge_d / 2.0,
                radius: badge_d / 2.0,
                number,
            });
        }
        if !lines.is_empty() {
            b.layout.texts.push(TextItem {
                lines,
                x: x + indent,
                y: cy + (row_h - text_h) / 2.0,
                size: title_size,
                bold: true,
                color: c.text_color,
            });
        }
        b.bottom = b.bottom.max(cy + row_h);
        cy += row_h + small_gap;
    }

    let show_description = overrides
        .and_then(|l| l.show_description)
        .unwrap_or(c.show_descriptions)
        && !step.description.trim().is_empty();
    let below = c.description_placement == DescriptionPlacement::BelowImage;

    let describe = |b: &mut Builder, cy: &mut f32| {
        if show_description {
            let h = b.text(&step.description, x, *cy, w, font, false);
            *cy += h + small_gap;
        }
    };
    if !below {
        describe(b, &mut cy);
    }

    // Screenshot.
    let (iw, ih) = sizes
        .get(&step.id)
        .map(|&(a, b)| (a.max(1) as f32, b.max(1) as f32))
        .unwrap_or((w, (w * PLACEHOLDER_RATIO).round().max(1.0)));
    let limit = overrides
        .and_then(|l| l.max_width)
        .map_or(w, |m| (m as f32).min(w))
        .max(1.0);
    let scale = match c.image_scaling {
        ImageScaling::ShrinkToFit => (limit / iw).min(1.0),
        ImageScaling::FillWidth => limit / iw,
    };
    let (tw, th) = ((iw * scale).round().max(1.0), (ih * scale).round().max(1.0));
    let ix = x + match c.image_align {
        ImageAlign::Left => 0.0,
        ImageAlign::Center => ((w - tw) / 2.0).floor(),
        ImageAlign::Right => w - tw,
    };
    b.layout.images.push(ImageItem {
        step: step.id,
        rect: RectF {
            x: ix,
            y: cy.round(),
            w: tw,
            h: th,
        },
    });
    cy = cy.round() + th;
    b.bottom = b.bottom.max(cy);
    if below {
        cy += small_gap;
        describe(b, &mut cy);
        cy -= small_gap; // no trailing gap after the last part
    }
    let height = cy - y;
    b.layout.boxes.push(StepBox {
        step: step.id,
        number,
        rect: RectF { x, y, w, h: height },
    });
    height
}

fn to_paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.0[0], c.0[1], c.0[2], c.0[3]);
    p.anti_alias = true;
    p
}

fn fill_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: Rgba) {
    if let Some(r) = Rect::from_xywh(x, y, w, h) {
        pm.fill_rect(r, &to_paint(c), Transform::identity(), None);
    }
}

/// Draws a layout. `scale` shrinks or enlarges everything uniformly (the preview uses less
/// than 1; export uses 1). `images` supplies each step's finished screenshot (annotations
/// drawn, crop applied) when its turn comes, and may return `None` for a missing one.
/// Images are asked for one at a time and dropped right after drawing.
pub fn render_layout(
    layout: &ComposedLayout,
    scale: f32,
    images: &mut dyn FnMut(ItemId) -> Option<Image>,
) -> Result<Image, ComposeError> {
    render_layout_until(layout, scale, images, &|| false)
}

/// Like [`render_layout`], but asks `stop` before each screenshot and gives up with
/// [`ComposeError::Cancelled`] as soon as it says so, instead of drawing the rest.
pub fn render_layout_until(
    layout: &ComposedLayout,
    scale: f32,
    images: &mut dyn FnMut(ItemId) -> Option<Image>,
    stop: &dyn Fn() -> bool,
) -> Result<Image, ComposeError> {
    let s = scale.clamp(0.01, 4.0);
    let (w, h) = (
        (layout.width as f32 * s).ceil().max(1.0) as u32,
        (layout.height as f32 * s).ceil().max(1.0) as u32,
    );
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(ComposeError::TooLarge(w, h));
    }
    let mut pm = Pixmap::new(w, h).ok_or(ComposeError::Alloc)?;
    let bg = layout.background.0;
    pm.fill(tiny_skia::Color::from_rgba8(bg[0], bg[1], bg[2], bg[3]));

    for t in &layout.texts {
        text::draw_plain(
            &mut pm,
            &t.lines.join("\n"),
            t.size * s,
            t.bold,
            Point::new(t.x * s, t.y * s),
            t.color,
        );
    }
    for badge in &layout.badges {
        if let Some(path) = PathBuilder::from_circle(badge.cx * s, badge.cy * s, badge.radius * s) {
            pm.fill_path(
                &path,
                &to_paint(ACCENT),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        text::draw_centered(
            &mut pm,
            &badge.number.to_string(),
            badge.radius * s * 1.1,
            Point::new(badge.cx * s, badge.cy * s),
            Rgba::WHITE,
        );
    }
    for item in &layout.images {
        if stop() {
            return Err(ComposeError::Cancelled);
        }
        let (x0, y0) = ((item.rect.x * s).round(), (item.rect.y * s).round());
        let (x1, y1) = (
            ((item.rect.x + item.rect.w) * s).round(),
            ((item.rect.y + item.rect.h) * s).round(),
        );
        let (tw, th) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
        match images(item.step) {
            Some(img)
                if img.width > 0
                    && img.height > 0
                    && img.rgba.len() == img.width as usize * img.height as usize * 4 =>
            {
                let fitted = resample(&img, tw as u32, th as u32);
                draw_image(&mut pm, &fitted, x0, y0);
            }
            _ => {
                fill_rect(&mut pm, x0, y0, tw, th, Rgba([225, 225, 225, 255]));
                fill_rect(
                    &mut pm,
                    x0,
                    y0,
                    tw,
                    2.0_f32.min(th),
                    Rgba([190, 190, 190, 255]),
                );
                let size = (14.0 * s).max(8.0);
                text::draw_centered(
                    &mut pm,
                    &layout.missing_label,
                    size,
                    Point::new(x0 + tw / 2.0, y0 + th / 2.0),
                    Rgba([110, 110, 110, 255]),
                );
            }
        }
    }
    Ok(crate::finish(&pm, None))
}

fn draw_image(pm: &mut Pixmap, img: &Image, x: f32, y: f32) {
    let Some(mut src) = Pixmap::new(img.width, img.height) else {
        return;
    };
    crate::premultiply_into(&img.rgba, src.data_mut());
    let paint = PixmapPaint {
        quality: FilterQuality::Nearest,
        blend_mode: BlendMode::SourceOver,
        ..PixmapPaint::default()
    };
    pm.draw_pixmap(
        x as i32,
        y as i32,
        src.as_ref(),
        &paint,
        Transform::identity(),
        None,
    );
}

/// A step's finished screenshot size: the crop if there is one, else the original size.
pub fn cropped_size(original: (u32, u32), crop: Option<PixelRect>) -> (u32, u32) {
    crop.map_or(original, |c| (c.w, c.h))
}

#[cfg(test)]
mod tests;
