//! Guide-only settings: how the combined guide image is composed. Pure data, no I/O.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::Rgba;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    Vertical,
    Grid { columns: u32 },
}

/// How a screenshot is sized inside its slot in the combined image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageScaling {
    /// Shrink screenshots wider than the slot; leave smaller ones at their natural size.
    ShrinkToFit,
    /// Scale every screenshot to the full width of its slot.
    FillWidth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DescriptionPlacement {
    AboveImage,
    BelowImage,
}

/// How the combined guide image is laid out. Consumed by the composition
/// engine; every field has a default so older files stay readable when fields are added.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompositionSettings {
    pub layout: Layout,
    pub background: Rgba,
    pub margin: u32,
    pub spacing: u32,
    pub max_width: u32,
    pub show_numbers: bool,
    pub show_titles: bool,
    pub show_descriptions: bool,
    pub font_size: f32,
    pub image_scaling: ImageScaling,
    pub image_align: ImageAlign,
    pub description_placement: DescriptionPlacement,
    pub show_guide_title: bool,
    pub show_introduction: bool,
    pub show_final_notes: bool,
    pub text_color: Rgba,
    /// Settings this version does not know (written by a newer version); kept so saving
    /// does not lose them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for CompositionSettings {
    fn default() -> Self {
        Self {
            layout: Layout::Vertical,
            background: Rgba([255, 255, 255, 255]),
            margin: 32,
            spacing: 24,
            max_width: 1200,
            show_numbers: true,
            show_titles: true,
            show_descriptions: true,
            font_size: 20.0,
            image_scaling: ImageScaling::ShrinkToFit,
            image_align: ImageAlign::Left,
            description_placement: DescriptionPlacement::AboveImage,
            show_guide_title: true,
            show_introduction: true,
            show_final_notes: true,
            text_color: Rgba([32, 32, 32, 255]),
            extra: Map::new(),
        }
    }
}

impl CompositionSettings {
    pub const MARGIN: (u32, u32) = (0, 2_000);
    pub const SPACING: (u32, u32) = (0, 2_000);
    pub const MAX_WIDTH: (u32, u32) = (200, 10_000);
    pub const FONT_SIZE: (f32, f32) = (8.0, 120.0);
    pub const COLUMNS: (u32, u32) = (1, 8);

    /// Brings values a hand-edited or damaged file may hold back into the range the layout
    /// engine supports. Returns one sentence for each value that had to change.
    ///
    /// Colours (`background`, `text_color`) are not checked: any RGBA value is valid, so a fully
    /// transparent or background-coloured text is the user's choice and produces blank text.
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        let mut clamp = |name: &str, v: &mut u32, (lo, hi): (u32, u32)| {
            let fixed = (*v).clamp(lo, hi);
            if fixed != *v {
                notes.push(format!(
                    "composition.{name} {v} is outside {lo}-{hi}; using {fixed}"
                ));
                *v = fixed;
            }
        };
        clamp("margin", &mut self.margin, Self::MARGIN);
        clamp("spacing", &mut self.spacing, Self::SPACING);
        clamp("max_width", &mut self.max_width, Self::MAX_WIDTH);
        if let Layout::Grid { columns } = &mut self.layout {
            clamp("layout.columns", columns, Self::COLUMNS);
        }
        let (lo, hi) = Self::FONT_SIZE;
        if !self.font_size.is_finite() {
            notes.push("composition.font_size is not a number; using 20".to_string());
            self.font_size = 20.0;
        } else if !(lo..=hi).contains(&self.font_size) {
            let fixed = self.font_size.clamp(lo, hi);
            notes.push(format!(
                "composition.font_size {} is outside {lo}-{hi}; using {fixed}",
                self.font_size
            ));
            self.font_size = fixed;
        }
        notes
    }
}

/// Per-step overrides of the guide-wide composition settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepLayout {
    pub max_width: Option<u32>,
    pub show_description: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_leaves_good_settings_alone_and_explains_every_change() {
        let mut ok = CompositionSettings::default();
        assert!(ok.sanitize().is_empty());
        assert_eq!(ok, CompositionSettings::default());

        let mut bad = CompositionSettings {
            margin: 99_999,
            spacing: 3_000,
            max_width: 5,
            font_size: 900.0,
            layout: Layout::Grid { columns: 0 },
            ..CompositionSettings::default()
        };
        let notes = bad.sanitize();
        assert_eq!(notes.len(), 5, "{notes:?}");
        assert_eq!(
            (bad.margin, bad.spacing, bad.max_width, bad.font_size),
            (2_000, 2_000, 200, 120.0)
        );
        assert_eq!(bad.layout, Layout::Grid { columns: 1 });
        for what in [
            "margin",
            "spacing",
            "max_width",
            "font_size",
            "layout.columns",
        ] {
            assert!(notes.iter().any(|n| n.contains(what)), "{what}: {notes:?}");
        }
        assert!(
            notes
                .iter()
                .any(|n| n.contains("99999") && n.contains("2000")),
            "values are named: {notes:?}"
        );

        let mut nan = CompositionSettings {
            font_size: f32::NAN,
            ..CompositionSettings::default()
        };
        assert_eq!(nan.sanitize().len(), 1);
        assert_eq!(nan.font_size, 20.0);
        let mut wide_grid = CompositionSettings {
            layout: Layout::Grid { columns: 50 },
            ..CompositionSettings::default()
        };
        assert_eq!(wide_grid.sanitize().len(), 1);
        assert_eq!(wide_grid.layout, Layout::Grid { columns: 8 });
    }

    #[test]
    fn step_layouts_default_to_no_overrides() {
        let l = StepLayout::default();
        assert!(l.max_width.is_none() && l.show_description.is_none());
    }
}
