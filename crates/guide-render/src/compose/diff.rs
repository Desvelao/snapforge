//! Diff composition: a previous and a post screenshot side by side or stacked, with an optional
//! header (title and description) and a caption under each screenshot. Produces the same
//! [`ComposedLayout`] as a guide, so drawing and the preview work unchanged.

use std::collections::HashMap;

use guide_core::{DiffLayout, DiffSettings, ItemId, Project, RectF, Rgba};

use super::{
    wrap_text, ComposeError, ComposedLayout, ImageItem, StepBox, TextItem, DEFAULT_MISSING_LABEL,
    PLACEHOLDER_RATIO,
};
use crate::{text, MAX_PIXELS};

/// Widest the two screenshots may be together (horizontal) or each (vertical).
const MAX_CONTENT_W: f32 = 2_400.0;
const MIN_CONTENT_W: f32 = 200.0;

fn text_item(
    s: &str,
    x: f32,
    y: f32,
    width: f32,
    size: f32,
    bold: bool,
    color: Rgba,
) -> Option<(TextItem, f32)> {
    let slack = if bold { size / 18.0 } else { 0.0 };
    let lines = wrap_text(s, size, (width - slack).max(1.0));
    if lines.is_empty() {
        return None;
    }
    let h = lines.len() as f32 * text::line_height(size);
    Some((
        TextItem {
            lines,
            x,
            y,
            size,
            bold,
            color,
        },
        h,
    ))
}

/// Where every part of a diff goes. `sizes` gives each screenshot's final size in pixels (after
/// cropping); a screenshot without an entry gets a placeholder. The first two included items are
/// the previous and the post screenshot.
pub fn compute_diff_layout(
    project: &Project,
    sizes: &HashMap<ItemId, (u32, u32)>,
) -> Result<ComposedLayout, ComposeError> {
    let default_settings;
    let d = match project.diff() {
        Some(d) => d,
        None => {
            default_settings = DiffSettings::default();
            &default_settings
        }
    };
    let margin = d.margin.min(2_000) as f32;
    let gap = d.spacing.min(2_000) as f32;
    let font = d.font_size.clamp(8.0, 120.0);
    let small_gap = font * 0.5;

    let items: Vec<_> = project.included_items().take(Project::DIFF_ITEMS).collect();
    let natural: Vec<(ItemId, f32, f32)> = items
        .iter()
        .map(|it| {
            let (w, h) = sizes.get(&it.id).map_or_else(
                || (800.0, (800.0 * PLACEHOLDER_RATIO).round()),
                |&(w, h)| (w.max(1) as f32, h.max(1) as f32),
            );
            (it.id, w, h)
        })
        .collect();

    // Scaled sizes: equal height side by side, equal width stacked; never enlarged past the
    // smaller original, and shrunk to fit the content limit.
    let horizontal = d.layout == DiffLayout::Horizontal;
    let mut scaled: Vec<(f32, f32)> = if horizontal {
        let h = natural.iter().map(|n| n.2).fold(f32::MAX, f32::min);
        natural.iter().map(|n| (n.1 * h / n.2, h)).collect()
    } else {
        let w = natural.iter().map(|n| n.1).fold(f32::MAX, f32::min);
        natural.iter().map(|n| (w, n.2 * w / n.1)).collect()
    };
    let used_w = |s: &[(f32, f32)]| {
        if horizontal {
            s.iter().map(|x| x.0).sum::<f32>() + gap * s.len().saturating_sub(1) as f32
        } else {
            s.iter().map(|x| x.0).fold(0.0, f32::max)
        }
    };
    let too_wide = used_w(&scaled);
    if too_wide > MAX_CONTENT_W {
        let fixed = if horizontal {
            gap * scaled.len().saturating_sub(1) as f32
        } else {
            0.0
        };
        let k = (MAX_CONTENT_W - fixed).max(1.0) / (too_wide - fixed).max(1.0);
        for s in &mut scaled {
            *s = (s.0 * k, s.1 * k);
        }
    }
    for s in &mut scaled {
        *s = (s.0.round().max(1.0), s.1.round().max(1.0));
    }
    let content_w = used_w(&scaled).max(MIN_CONTENT_W);
    let canvas_w = (content_w + 2.0 * margin).ceil();

    let mut layout = ComposedLayout {
        width: canvas_w as u32,
        height: 0,
        background: d.background,
        texts: Vec::new(),
        badges: Vec::new(),
        images: Vec::new(),
        boxes: Vec::new(),
        missing_label: DEFAULT_MISSING_LABEL.to_string(),
    };
    let mut y = margin;
    let mut bottom = margin;

    if d.show_header {
        let title = if d.title.trim().is_empty() {
            project.name.as_str()
        } else {
            d.title.as_str()
        };
        if let Some((t, h)) = text_item(title, margin, y, content_w, font * 1.8, true, d.text_color)
        {
            layout.texts.push(t);
            y += h + small_gap;
        }
        if let Some((t, h)) = text_item(
            &d.description,
            margin,
            y,
            content_w,
            font,
            false,
            d.text_color,
        ) {
            layout.texts.push(t);
            y += h + small_gap;
        }
        if !layout.texts.is_empty() {
            y += gap - small_gap.min(gap);
        }
        bottom = bottom.max(y);
    }

    let mut x = margin;
    for (i, (item, (&(id, _, _), &(w, h)))) in
        items.iter().zip(natural.iter().zip(&scaled)).enumerate()
    {
        let top = y.round();
        layout.images.push(ImageItem {
            step: id,
            rect: RectF { x, y: top, w, h },
        });
        let mut end = top + h;
        if d.show_captions {
            if let Some((t, th)) = text_item(
                &item.caption,
                x,
                end + small_gap,
                w,
                font,
                false,
                d.text_color,
            ) {
                layout.texts.push(t);
                end += small_gap + th;
            }
        }
        layout.boxes.push(StepBox {
            step: id,
            number: i as u32 + 1,
            rect: RectF {
                x,
                y: top,
                w,
                h: end - top,
            },
        });
        bottom = bottom.max(end);
        if horizontal {
            x += w + gap;
        } else {
            y = end + gap;
        }
    }

    let height = (bottom + margin).ceil();
    if u64::from(canvas_w as u32) * height as u64 > MAX_PIXELS || height > f32::from(u16::MAX) * 4.0
    {
        return Err(ComposeError::TooLarge(
            canvas_w as u32,
            height.min(u32::MAX as f32) as u32,
        ));
    }
    layout.height = height as u32;
    Ok(layout)
}
