//! The pieces the editor's side panels are made of: tool tiles with painted icons, colour
//! swatches and labelled sliders. They only draw and report clicks; the editor decides what to do.

use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Painter, Pos2, Rect,
    Shape, Stroke, StrokeKind,
};
use guide_core::Rgba;

use super::Tool;
use crate::i18n::tr;
use crate::theme;

/// The colours offered as swatches, with the name a screen reader announces.
pub fn swatches() -> [(Rgba, &'static str); 5] {
    [
        (Rgba::RED, tr("Red")),
        (Rgba([255, 214, 64, 255]), tr("Yellow")),
        (Rgba([60, 200, 140, 255]), tr("Green")),
        (Rgba([80, 150, 255, 255]), tr("Blue")),
        (Rgba([255, 255, 255, 255]), tr("White")),
    ]
}

/// A tool tile: icon over name, the shortcut letter in the corner. `selected` gives it the
/// accent outline, tint and text. Returns the response (clicked = choose the tool).
pub fn tool_tile(ui: &mut egui::Ui, tool: Tool, selected: bool, width: f32) -> egui::Response {
    let pal = theme::palette(ui.ctx());
    let name = format!("{} ({})", tool.label(), tool.key().name()); // i18n-ok
    let (rect, response) = ui.allocate_exact_size(vec2(width, 62.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &name)
    });
    let (fill, stroke) = if selected {
        (pal.selected_tint, Stroke::new(1.5, pal.accent))
    } else if response.hovered() {
        (pal.row_hover, Stroke::new(1.0, pal.control_border))
    } else {
        (pal.row, Stroke::new(1.0, pal.control_border))
    };
    let fg = if selected { pal.accent } else { pal.text };
    let painter = ui.painter();
    let radius = CornerRadius::same(theme::WIDGET_RADIUS);
    painter.rect_filled(rect, radius, fill);
    painter.rect_stroke(rect, radius, stroke, StrokeKind::Inside);
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(theme::WIDGET_RADIUS + 2),
            ui.visuals().selection.stroke,
            StrokeKind::Outside,
        );
    }
    paint_icon(painter, tool, pos2(rect.center().x, rect.top() + 21.0), fg);
    painter.text(
        pos2(rect.center().x, rect.bottom() - 13.0),
        Align2::CENTER_CENTER,
        tool.label(),
        FontId::proportional(12.0),
        fg,
    );
    painter.text(
        pos2(rect.right() - 7.0, rect.top() + 8.0),
        Align2::RIGHT_CENTER,
        tool.key().name(),
        FontId::monospace(10.0),
        if selected { pal.accent } else { pal.muted },
    );
    response.on_hover_text(tool.hint())
}

/// A tool's icon in `color`, centred on `c`, drawn with simple strokes inside an 18 px square.
pub fn paint_icon(p: &Painter, tool: Tool, c: Pos2, color: Color32) {
    let st = Stroke::new(1.6, color);
    let at = |x: f32, y: f32| pos2(c.x + x, c.y + y);
    match tool {
        Tool::Select => {
            let outline = vec![
                at(-4.5, -8.0),
                at(-4.5, 6.0),
                at(-1.2, 3.0),
                at(1.4, 8.0),
                at(3.6, 7.0),
                at(1.2, 2.2),
                at(5.5, 2.0),
            ];
            p.add(Shape::closed_line(outline, st));
        }
        Tool::Pen => {
            p.line_segment([at(-5.5, 6.0), at(4.5, -4.0)], Stroke::new(2.4, color));
            p.line_segment([at(3.0, -6.0), at(6.5, -2.5)], st);
            p.line_segment([at(-7.0, 8.0), at(-3.5, 7.5)], st);
        }
        Tool::Highlighter => {
            p.line_segment(
                [at(-5.0, 3.0), at(3.0, -5.0)],
                Stroke::new(5.0, color.gamma_multiply(0.55)),
            );
            p.line_segment([at(-7.0, 8.0), at(7.0, 8.0)], st);
        }
        Tool::Line => {
            p.line_segment([at(-7.0, 7.0), at(7.0, -7.0)], st);
        }
        Tool::Arrow => {
            p.line_segment([at(-7.0, 7.0), at(7.0, -7.0)], st);
            p.line_segment([at(7.0, -7.0), at(1.0, -7.0)], st);
            p.line_segment([at(7.0, -7.0), at(7.0, -1.0)], st);
        }
        Tool::Rectangle => {
            p.rect_stroke(
                Rect::from_center_size(c, vec2(15.0, 10.0)),
                CornerRadius::same(2),
                st,
                StrokeKind::Middle,
            );
        }
        Tool::Ellipse => {
            p.add(Shape::ellipse_stroke(c, vec2(7.5, 5.0), st));
        }
        Tool::Text => {
            p.text(
                c,
                Align2::CENTER_CENTER,
                "T", // i18n-ok
                FontId::new(18.0, theme::strong_family()),
                color,
            );
        }
        Tool::Marker => {
            p.circle_stroke(c, 7.5, st);
            p.text(
                c,
                Align2::CENTER_CENTER,
                "1", // i18n-ok
                FontId::monospace(10.0),
                color,
            );
        }
        Tool::Blur => {
            p.circle_stroke(at(0.0, 2.5), 5.0, st);
            p.line_segment([at(0.0, -8.0), at(-4.2, -1.0)], st);
            p.line_segment([at(0.0, -8.0), at(4.2, -1.0)], st);
        }
        Tool::Crop => {
            p.line_segment([at(-3.0, -8.0), at(-3.0, 3.0)], st);
            p.line_segment([at(-3.0, 3.0), at(8.0, 3.0)], st);
            p.line_segment([at(3.0, 8.0), at(3.0, -3.0)], st);
            p.line_segment([at(3.0, -3.0), at(-8.0, -3.0)], st);
        }
        Tool::Eraser => {
            let outline = vec![at(-7.0, 2.0), at(-1.0, -6.0), at(7.0, 0.0), at(1.0, 8.0)];
            p.add(Shape::closed_line(outline, st));
            p.line_segment([at(-3.5, 5.0), at(4.5, 3.5)], st);
        }
    }
}

/// The colour swatches. `current` is the colour in use (its alpha is ignored when comparing);
/// returns the colour that was clicked, if any.
pub fn swatch_row(ui: &mut egui::Ui, current: Rgba) -> Option<Rgba> {
    let pal = theme::palette(ui.ctx());
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
        for (colour, name) in swatches() {
            let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::click());
            let selected = colour.0[..3] == current.0[..3];
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, name)
            });
            let fill = Color32::from_rgb(colour.0[0], colour.0[1], colour.0[2]);
            let p = ui.painter();
            p.circle_filled(rect.center(), 12.0, fill);
            p.circle_stroke(rect.center(), 12.0, Stroke::new(1.0, pal.control_border));
            if selected {
                p.circle_stroke(rect.center(), 14.5, Stroke::new(2.0, pal.text));
            }
            if response.has_focus() {
                p.circle_stroke(rect.center(), 16.0, ui.visuals().selection.stroke);
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                picked = Some(colour);
            }
        }
    });
    picked
}

/// A caption with its value on the right, over a slider that spans the panel. Returns the
/// slider's response; `value_text` is the value as shown ("4 px").
pub fn slider_row<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    caption: &str,
    value_text: &str,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
) -> egui::Response {
    let pal = theme::palette(ui.ctx());
    let label = ui
        .horizontal(|ui| {
            let label = theme::section_label(ui, caption);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(value_text)
                        .monospace()
                        .size(13.0)
                        .color(pal.text),
                );
            });
            label
        })
        .inner;
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = ui.available_width();
        // The rail is drawn with the widget fill, which is the panel colour here.
        let v = ui.visuals_mut();
        v.widgets.inactive.bg_fill = pal.control_border;
        v.widgets.hovered.bg_fill = pal.control_border;
        v.widgets.active.bg_fill = pal.control_border;
        v.selection.bg_fill = pal.accent;
        ui.add(egui::Slider::new(value, range).show_value(false))
            .labelled_by(label.id)
    })
    .inner
}
