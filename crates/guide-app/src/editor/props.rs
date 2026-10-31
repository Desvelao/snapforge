//! The editor's current tool settings, and how they map onto annotations.

use eframe::egui::Color32;
use guide_core::{Annotation, Kind, ObscureMode, Rgba, Style};
use guide_storage::EditorOptions;

/// Alpha of highlighter strokes; a fully opaque highlighter would hide the text.
pub const HIGHLIGHT_ALPHA: u8 = 110;

pub fn to_color32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3])
}

pub fn from_color32(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba([r, g, b, a])
}

#[derive(Clone, Debug, PartialEq)]
pub struct Props {
    pub color: Rgba,
    pub width: f32,
    /// The highlighter has its own, much wider, stroke width.
    pub highlighter_width: f32,
    pub fill_on: bool,
    pub fill: Rgba,
    pub text_size: f32,
    pub bold: bool,
    pub obscure_mode: ObscureMode,
    pub obscure_strength: u32,
    pub marker_radius: f32,
}

impl Default for Props {
    fn default() -> Self {
        Self {
            color: Rgba::RED,
            width: 4.0,
            highlighter_width: 22.0,
            fill_on: false,
            fill: Rgba([255, 255, 255, 255]),
            text_size: 28.0,
            bold: false,
            obscure_mode: ObscureMode::Pixelate,
            obscure_strength: 14,
            marker_radius: 16.0,
        }
    }
}

impl From<&EditorOptions> for Props {
    fn from(o: &EditorOptions) -> Self {
        Self {
            color: o.color,
            width: o.width,
            highlighter_width: o.highlighter_width,
            fill_on: o.fill_on,
            fill: o.fill,
            text_size: o.text_size,
            bold: o.bold,
            obscure_mode: o.obscure_mode,
            obscure_strength: o.obscure_strength,
            marker_radius: o.marker_radius,
        }
    }
}

impl From<&Props> for EditorOptions {
    fn from(p: &Props) -> Self {
        Self {
            color: p.color,
            width: p.width,
            highlighter_width: p.highlighter_width,
            fill_on: p.fill_on,
            fill: p.fill,
            text_size: p.text_size,
            bold: p.bold,
            obscure_mode: p.obscure_mode,
            obscure_strength: p.obscure_strength,
            marker_radius: p.marker_radius,
        }
    }
}

impl Props {
    pub fn fill_value(&self) -> Option<Rgba> {
        self.fill_on.then_some(self.fill)
    }

    /// Style for a new annotation of the given kind.
    pub fn style_for(&self, highlighter: bool) -> Style {
        if highlighter {
            Style {
                color: Rgba([
                    self.color.0[0],
                    self.color.0[1],
                    self.color.0[2],
                    HIGHLIGHT_ALPHA,
                ]),
                width: self.highlighter_width,
                fill: None,
            }
        } else {
            Style {
                color: self.color,
                width: self.width,
                fill: self.fill_value(),
            }
        }
    }

    /// Writes these settings into an existing annotation (restyling a selection).
    pub fn apply(&self, a: &mut Annotation) {
        a.style = self.style_for(matches!(a.kind, Kind::Highlighter { .. }));
        match &mut a.kind {
            Kind::Text { size, bold, .. } => (*size, *bold) = (self.text_size, self.bold),
            Kind::Marker { radius, .. } => *radius = self.marker_radius,
            Kind::Obscure { mode, strength, .. } => {
                (*mode, *strength) = (self.obscure_mode, self.obscure_strength)
            }
            _ => {}
        }
    }

    /// Takes the settings shown in the controls from a selected annotation.
    pub fn load(&mut self, a: &Annotation) {
        let c = a.style.color.0;
        if matches!(a.kind, Kind::Highlighter { .. }) {
            self.color = Rgba([c[0], c[1], c[2], 255]);
            self.highlighter_width = a.style.width;
        } else {
            self.color = a.style.color;
            self.width = a.style.width;
            if let Some(f) = a.style.fill {
                self.fill_on = true;
                self.fill = f;
            } else {
                self.fill_on = false;
            }
        }
        match &a.kind {
            Kind::Text { size, bold, .. } => (self.text_size, self.bold) = (*size, *bold),
            Kind::Marker { radius, .. } => self.marker_radius = *radius,
            Kind::Obscure { mode, strength, .. } => {
                (self.obscure_mode, self.obscure_strength) = (*mode, *strength)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guide_core::{AnnotationId, Point, RectF};

    fn ann(kind: Kind, style: Style) -> Annotation {
        Annotation {
            id: AnnotationId(1),
            kind,
            style,
        }
    }

    #[test]
    fn highlighter_style_is_translucent_and_wide() {
        let p = Props {
            color: Rgba([255, 235, 0, 255]),
            ..Props::default()
        };
        let s = p.style_for(true);
        assert_eq!(s.color.0[3], HIGHLIGHT_ALPHA);
        assert_eq!(s.width, 22.0);
        assert_eq!(p.style_for(false).color.0[3], 255);
    }

    #[test]
    fn fill_only_applies_when_enabled() {
        let mut p = Props::default();
        assert_eq!(p.style_for(false).fill, None);
        p.fill_on = true;
        assert_eq!(p.style_for(false).fill, Some(p.fill));
    }

    #[test]
    fn apply_restyles_and_sets_kind_specific_fields() {
        let p = Props {
            text_size: 40.0,
            bold: true,
            color: Rgba([1, 2, 3, 255]),
            width: 9.0,
            ..Props::default()
        };
        let mut a = ann(
            Kind::Text {
                at: Point::new(0.0, 0.0),
                text: "x".into(),
                size: 10.0,
                bold: false,
            },
            Style::default(),
        );
        p.apply(&mut a);
        assert_eq!(a.style.color, Rgba([1, 2, 3, 255]));
        assert!(matches!(a.kind, Kind::Text { size, bold: true, .. } if size == 40.0));

        let mut o = ann(
            Kind::Obscure {
                rect: RectF {
                    x: 0.0,
                    y: 0.0,
                    w: 5.0,
                    h: 5.0,
                },
                mode: ObscureMode::Blur,
                strength: 3,
            },
            Style::default(),
        );
        Props {
            obscure_mode: ObscureMode::Pixelate,
            obscure_strength: 20,
            ..Props::default()
        }
        .apply(&mut o);
        assert!(matches!(
            o.kind,
            Kind::Obscure {
                mode: ObscureMode::Pixelate,
                strength: 20,
                ..
            }
        ));
    }

    #[test]
    fn load_then_apply_is_the_identity_for_each_kind() {
        let samples = [
            ann(
                Kind::Rectangle {
                    rect: RectF {
                        x: 0.0,
                        y: 0.0,
                        w: 5.0,
                        h: 5.0,
                    },
                },
                Style {
                    color: Rgba([9, 8, 7, 255]),
                    width: 6.0,
                    fill: Some(Rgba([1, 1, 1, 255])),
                },
            ),
            ann(
                Kind::Highlighter {
                    points: vec![Point::new(0.0, 0.0)],
                },
                Style {
                    color: Rgba([255, 235, 0, HIGHLIGHT_ALPHA]),
                    width: 30.0,
                    fill: None,
                },
            ),
            ann(
                Kind::Marker {
                    center: Point::new(1.0, 1.0),
                    number: 4,
                    radius: 21.0,
                },
                Style {
                    color: Rgba([0, 0, 255, 255]),
                    width: 4.0,
                    fill: None,
                },
            ),
            ann(
                Kind::Text {
                    at: Point::new(0.0, 0.0),
                    text: "t".into(),
                    size: 33.0,
                    bold: true,
                },
                Style {
                    color: Rgba([5, 5, 5, 255]),
                    width: 4.0,
                    fill: None,
                },
            ),
        ];
        // Kinds that take only the common style, and the two obscure modes.
        let plain = |color: [u8; 4], width: f32, fill: Option<Rgba>| Style {
            color: Rgba(color),
            width,
            fill,
        };
        let (from, to) = (Point::new(1.0, 2.0), Point::new(30.0, 40.0));
        let rect = RectF {
            x: 2.0,
            y: 3.0,
            w: 20.0,
            h: 10.0,
        };
        let more = [
            ann(Kind::Line { from, to }, plain([10, 20, 30, 255], 3.0, None)),
            ann(
                Kind::Arrow { from, to },
                plain([40, 50, 60, 255], 7.0, None),
            ),
            ann(
                Kind::Pen {
                    points: vec![from, to],
                },
                plain([70, 80, 90, 255], 2.0, None),
            ),
            ann(
                Kind::Ellipse { rect },
                plain([1, 2, 3, 255], 5.0, Some(Rgba([200, 100, 50, 255]))),
            ),
            ann(
                Kind::Obscure {
                    rect,
                    mode: ObscureMode::Blur,
                    strength: 77,
                },
                plain([0, 0, 0, 255], 4.0, None),
            ),
            ann(
                Kind::Obscure {
                    rect,
                    mode: ObscureMode::Pixelate,
                    strength: 9,
                },
                plain([0, 0, 0, 255], 4.0, None),
            ),
        ];
        for a in samples.into_iter().chain(more) {
            let mut p = Props::default();
            p.load(&a);
            let mut b = a.clone();
            p.apply(&mut b);
            assert_eq!(b, a);
        }
    }

    #[test]
    fn props_and_profile_editor_options_convert_both_ways_without_loss() {
        assert_eq!(
            EditorOptions::from(&Props::default()),
            EditorOptions::default()
        );
        let p = Props {
            color: Rgba([1, 2, 3, 255]),
            width: 7.0,
            highlighter_width: 30.0,
            fill_on: true,
            fill: Rgba([9, 9, 9, 255]),
            text_size: 50.0,
            bold: true,
            obscure_mode: ObscureMode::Blur,
            obscure_strength: 33,
            marker_radius: 22.0,
        };
        assert_eq!(Props::from(&EditorOptions::from(&p)), p);
    }

    #[test]
    fn color_conversion_round_trips() {
        for c in [
            Rgba([0, 0, 0, 255]),
            Rgba([255, 235, 0, 110]),
            Rgba([12, 34, 56, 255]),
        ] {
            let back = from_color32(to_color32(c));
            // Translucent colours lose at most a rounding step through premultiplied egui storage.
            for (a, b) in back.0.iter().zip(c.0) {
                assert!(a.abs_diff(b) <= 2, "{back:?} vs {c:?}");
            }
        }
    }
}
