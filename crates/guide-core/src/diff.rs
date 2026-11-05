//! Diff-only settings: a before/after pair composed into one image. Pure data, no I/O.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::Rgba;

/// How the previous and the post screenshot are arranged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLayout {
    /// Before on the left, after on the right.
    #[default]
    Horizontal,
    /// Before on top, after below.
    Vertical,
}

/// Settings of a diff project. Every field has a default so older files stay readable when
/// fields are added.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffSettings {
    pub layout: DiffLayout,
    /// Show the header (title and description) above the screenshots.
    pub show_header: bool,
    pub title: String,
    pub description: String,
    /// Show each screenshot's caption under it.
    pub show_captions: bool,
    pub background: Rgba,
    pub text_color: Rgba,
    pub margin: u32,
    pub spacing: u32,
    pub font_size: f32,
    /// Settings this version does not know; kept so saving does not lose them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for DiffSettings {
    fn default() -> Self {
        Self {
            layout: DiffLayout::Horizontal,
            show_header: true,
            title: String::new(),
            description: String::new(),
            show_captions: true,
            background: Rgba([255, 255, 255, 255]),
            text_color: Rgba([32, 32, 32, 255]),
            margin: 32,
            spacing: 24,
            font_size: 20.0,
            extra: Map::new(),
        }
    }
}

impl DiffSettings {
    pub const MARGIN: (u32, u32) = (0, 2_000);
    pub const SPACING: (u32, u32) = (0, 2_000);
    pub const FONT_SIZE: (f32, f32) = (8.0, 120.0);

    /// Brings values a hand-edited or damaged file may hold back into the supported range.
    /// Returns one sentence for each value that had to change.
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        let mut clamp = |name: &str, v: &mut u32, (lo, hi): (u32, u32)| {
            let fixed = (*v).clamp(lo, hi);
            if fixed != *v {
                notes.push(format!(
                    "diff.{name} {v} is outside {lo}-{hi}; using {fixed}"
                ));
                *v = fixed;
            }
        };
        clamp("margin", &mut self.margin, Self::MARGIN);
        clamp("spacing", &mut self.spacing, Self::SPACING);
        let (lo, hi) = Self::FONT_SIZE;
        if !self.font_size.is_finite() {
            notes.push("diff.font_size is not a number; using 20".to_string());
            self.font_size = 20.0;
        } else if !(lo..=hi).contains(&self.font_size) {
            let fixed = self.font_size.clamp(lo, hi);
            notes.push(format!(
                "diff.font_size {} is outside {lo}-{hi}; using {fixed}",
                self.font_size
            ));
            self.font_size = fixed;
        }
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_leaves_good_diff_settings_alone_and_explains_every_change() {
        let mut ok = DiffSettings::default();
        assert!(ok.sanitize().is_empty());
        assert_eq!(ok, DiffSettings::default());

        let mut bad = DiffSettings {
            margin: 99_999,
            spacing: 3_000,
            font_size: f32::NAN,
            ..DiffSettings::default()
        };
        assert_eq!(bad.sanitize().len(), 3);
        assert_eq!(
            (bad.margin, bad.spacing, bad.font_size),
            (2_000, 2_000, 20.0)
        );
    }

    #[test]
    fn a_diff_reads_back_with_unknown_fields_and_defaults_kept() {
        let s: DiffSettings = serde_json::from_str(r#"{"layout":"vertical","future":1}"#).unwrap();
        assert_eq!(s.layout, DiffLayout::Vertical);
        assert!(s.show_header && s.show_captions);
        assert!(s.extra.contains_key("future"));
    }
}
