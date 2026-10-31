//! Colours and styling for the light and dark themes.
//!
//! Every colour the interface draws itself comes from [`Palette`], and the tests below check the
//! text and focus colours against the WCAG contrast thresholds (4.5:1 for text, 3:1 for the focus
//! ring and other interface parts), so a change to a colour that breaks readability fails the build.

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke};
use guide_storage::ThemeChoice;

/// Corner radius of buttons, fields and other widgets.
pub const WIDGET_RADIUS: u8 = 8;
/// Corner radius of cards and popups.
pub const CARD_RADIUS: u8 = 12;
/// Height of the main toggles (capture mode, preview mode).
pub const TOGGLE_HEIGHT: f32 = 44.0;

thread_local! {
    /// Whether [`install_fonts`] ran on this (the UI) thread; egui panics when a family that was
    /// never registered is used, so [`strong_family`] falls back until then (tests draw without it).
    static FONTS_INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The bold UI face: Inter SemiBold, used by headings and the accent button.
pub fn strong_family() -> FontFamily {
    if FONTS_INSTALLED.get() {
        FontFamily::Name("strong".into())
    } else {
        FontFamily::Proportional
    }
}

/// The colours used by the interface that the app draws itself (egui's own widgets are styled
/// from the same values in [`apply`]).
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    pub background: Color32,
    pub panel: Color32,
    pub text: Color32,
    /// Secondary text; still readable.
    pub muted: Color32,
    pub error: Color32,
    /// Positive state, e.g. "All changes saved".
    pub success: Color32,
    pub accent: Color32,
    /// Text and icons drawn on an [`accent`](Self::accent) fill.
    pub on_accent: Color32,
    /// The ring drawn around the widget that has keyboard focus.
    pub focus: Color32,
    pub row: Color32,
    pub row_hover: Color32,
    pub row_selected: Color32,
    pub row_selected_text: Color32,
    /// Fill of a selected tool tile or segment. Quieter than [`row_selected`](Self::row_selected):
    /// these also have the accent outline and accent text, so the state does not rest on the fill.
    pub selected_tint: Color32,
    pub thumb_background: Color32,
    /// Hairline around cards and panels. Decorative: it only separates areas that are also
    /// told apart by their fill, so it is not held to the 3:1 contrast of [`control_border`](Self::control_border).
    pub border: Color32,
    /// Outline of buttons, fields and toggles, which must stay visible against the panel (3:1).
    pub control_border: Color32,
    /// Behind the screenshot in the editor and the composition preview.
    pub canvas: Color32,
}

pub const DARK: Palette = Palette {
    dark: true,
    background: Color32::from_rgb(0x0E, 0x10, 0x13),
    panel: Color32::from_rgb(0x14, 0x16, 0x1A),
    text: Color32::from_rgb(0xE8, 0xEA, 0xEE),
    muted: Color32::from_rgb(0x9B, 0xA2, 0xAE),
    error: Color32::from_rgb(0xFF, 0x7A, 0x6E),
    success: Color32::from_rgb(0x4A, 0xD6, 0xA0),
    accent: Color32::from_rgb(0xFF, 0x8A, 0x3D),
    on_accent: Color32::from_rgb(0x1A, 0x10, 0x05),
    focus: Color32::from_rgb(255, 214, 90),
    row: Color32::from_rgb(0x1B, 0x1E, 0x24),
    row_hover: Color32::from_rgb(0x25, 0x2A, 0x32),
    row_selected: Color32::from_rgb(0x45, 0x30, 0x1B),
    row_selected_text: Color32::from_rgb(0xFF, 0x8A, 0x3D),
    selected_tint: Color32::from_rgb(0x2A, 0x1F, 0x16),
    thumb_background: Color32::from_rgb(0x0E, 0x10, 0x13),
    border: Color32::from_rgb(0x2C, 0x31, 0x3B),
    control_border: Color32::from_rgb(0x6B, 0x73, 0x82),
    canvas: Color32::from_rgb(0x10, 0x12, 0x16),
};

pub const LIGHT: Palette = Palette {
    dark: false,
    background: Color32::from_rgb(246, 246, 249),
    panel: Color32::from_rgb(255, 255, 255),
    text: Color32::from_rgb(26, 26, 32),
    muted: Color32::from_rgb(86, 86, 98),
    error: Color32::from_rgb(176, 24, 24),
    success: Color32::from_rgb(0, 122, 80),
    accent: Color32::from_rgb(0, 84, 190),
    on_accent: Color32::from_rgb(255, 255, 255),
    focus: Color32::from_rgb(0, 70, 170),
    row: Color32::from_rgb(236, 236, 242),
    row_hover: Color32::from_rgb(222, 224, 236),
    row_selected: Color32::from_rgb(184, 208, 250),
    row_selected_text: Color32::from_rgb(10, 20, 50),
    selected_tint: Color32::from_rgb(184, 208, 250),
    thumb_background: Color32::from_rgb(214, 214, 222),
    border: Color32::from_rgb(214, 214, 224),
    control_border: Color32::from_rgb(120, 120, 135),
    canvas: Color32::from_rgb(200, 200, 208),
};

/// The palette matching the theme egui is currently showing.
pub fn palette(ctx: &egui::Context) -> Palette {
    if ctx.style().visuals.dark_mode {
        DARK
    } else {
        LIGHT
    }
}

/// Whether the dark palette applies for `choice`. `System` follows the desktop when it reports
/// a preference (many X11 desktops do not) and is dark otherwise.
pub fn use_dark(choice: ThemeChoice, system: Option<egui::Theme>) -> bool {
    match choice {
        ThemeChoice::Dark => true,
        ThemeChoice::Light => false,
        ThemeChoice::System => system.is_none_or(|t| t == egui::Theme::Dark),
    }
}

fn visuals(p: &Palette) -> egui::Visuals {
    let mut v = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.menu_corner_radius = CornerRadius::same(WIDGET_RADIUS);
    v.extreme_bg_color = p.background; // text fields
    v.faint_bg_color = p.row;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.error_fg_color = p.error;
    v.selection.bg_fill = p.row_selected;
    // The same ring is drawn around the widget that has keyboard focus: thick and high contrast.
    v.selection.stroke = Stroke::new(2.0, p.focus);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.fg_stroke.color = p.text;
        w.corner_radius = CornerRadius::same(WIDGET_RADIUS);
    }
    v.widgets.noninteractive.bg_fill = p.row;
    v.widgets.noninteractive.weak_bg_fill = p.row;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.bg_fill = p.row;
    v.widgets.inactive.weak_bg_fill = p.row;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.control_border);
    v.widgets.hovered.bg_fill = p.row_hover;
    v.widgets.hovered.weak_bg_fill = p.row_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.5, p.accent);
    v.widgets.active.bg_fill = p.row_selected;
    v.widgets.active.weak_bg_fill = p.row_selected;
    v.widgets.active.bg_stroke = Stroke::new(2.0, p.accent);
    v.widgets.open.bg_fill = p.row_hover;
    v.widgets.open.weak_bg_fill = p.row_hover;
    v
}

/// Registers Inter (UI text) and JetBrains Mono (shortcuts and paths) ahead of egui's built-in
/// faces, which stay behind them as fallbacks for symbols and scripts the two do not cover.
/// Call once at start-up.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (
            "Inter",
            &include_bytes!("../assets/fonts/Inter-Regular.ttf")[..],
        ),
        (
            "Inter-SemiBold",
            &include_bytes!("../assets/fonts/Inter-SemiBold.ttf")[..],
        ),
        (
            "JetBrains Mono",
            &include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")[..],
        ),
    ] {
        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(FontData::from_static(bytes)),
        );
    }
    let family = |fonts: &mut FontDefinitions, f: FontFamily, first: &str| {
        let list = fonts.families.entry(f).or_default();
        list.insert(0, first.to_owned());
    };
    family(&mut fonts, FontFamily::Proportional, "Inter");
    family(&mut fonts, FontFamily::Monospace, "JetBrains Mono");
    let mut strong = vec!["Inter-SemiBold".to_owned()];
    strong.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts
        .families
        .insert(FontFamily::Name("strong".into()), strong);
    ctx.set_fonts(fonts);
    FONTS_INSTALLED.set(true);
}

/// Applies the theme. Call when it changes, not every frame.
pub fn apply(ctx: &egui::Context, choice: ThemeChoice) {
    let dark = use_dark(choice, ctx.system_theme());
    ctx.set_visuals(visuals(if dark { &DARK } else { &LIGHT }));
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    ctx.style_mut(|s| {
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = s.spacing.interact_size.y.max(28.0);
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
    });
}

/// How prominent a [`Btn`] is. Only one accent button should be on screen at a time.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Emphasis {
    Accent,
    Secondary,
}

/// A large button with an optional monospace chip (usually a shortcut), drawn by hand so the
/// accent fill and text colours are not changed by egui's `override_text_color`.
pub struct Btn<'a> {
    pub label: &'a str,
    pub chip: Option<&'a str>,
    /// Muted text after the label, e.g. "one shot per click".
    pub note: Option<&'a str>,
    /// The name a screen reader announces; the visible label may be shorter.
    pub name: &'a str,
    pub emphasis: Emphasis,
    pub fill_width: bool,
    pub height: f32,
    /// Space left and right of the content.
    pub padding: f32,
}

impl<'a> Btn<'a> {
    pub fn new(label: &'a str, emphasis: Emphasis) -> Self {
        Self {
            label,
            chip: None,
            note: None,
            name: label,
            emphasis,
            fill_width: false,
            height: 32.0,
            padding: 28.0,
        }
    }

    /// Less space around the content, for bars that hold many buttons.
    pub fn compact(mut self) -> Self {
        self.padding = 18.0;
        self
    }

    pub fn chip(mut self, chip: &'a str) -> Self {
        self.chip = Some(chip);
        self
    }

    pub fn note(mut self, note: &'a str) -> Self {
        self.note = Some(note);
        self
    }

    pub fn name(mut self, name: &'a str) -> Self {
        self.name = name;
        self
    }

    pub fn full_width(mut self, height: f32) -> Self {
        self.fill_width = true;
        self.height = height;
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let pal = palette(ui.ctx());
        let enabled = ui.is_enabled();
        let accent = self.emphasis == Emphasis::Accent;
        let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let fg = dim(if accent { pal.on_accent } else { pal.text });
        let (label_font, size) = if accent {
            (strong_family(), 16.0)
        } else {
            (FontFamily::Proportional, 14.0)
        };
        let painter = ui.painter();
        let label = painter.layout_no_wrap(
            self.label.to_owned(),
            egui::FontId::new(size, label_font),
            fg,
        );
        let note = self.note.map(|n| {
            painter.layout_no_wrap(
                n.to_owned(),
                egui::FontId::proportional(13.0),
                dim(pal.muted),
            )
        });
        let chip = self
            .chip
            .map(|c| painter.layout_no_wrap(c.to_owned(), egui::FontId::monospace(12.0), fg));
        let gap = 8.0;
        let chip_w = chip.as_ref().map_or(0.0, |c| c.size().x + 12.0);
        let total = label.size().x
            + note.as_ref().map_or(0.0, |n| gap + n.size().x)
            + if chip.is_some() { gap + chip_w } else { 0.0 };
        let width = if self.fill_width {
            ui.available_width()
        } else {
            total + self.padding
        };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, self.height), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, self.name)
        });
        let hovered = response.hovered() || response.has_focus();
        let (fill, stroke) = match (accent, hovered) {
            (true, false) => (pal.accent, Stroke::NONE),
            (true, true) => (pal.accent.lerp_to_gamma(Color32::WHITE, 0.12), Stroke::NONE),
            (false, false) => (pal.panel, Stroke::new(1.0, pal.control_border)),
            (false, true) => (pal.row_hover, Stroke::new(1.0, pal.control_border)),
        };
        let radius = CornerRadius::same(WIDGET_RADIUS);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, dim(fill));
        if stroke != Stroke::NONE {
            painter.rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect.expand(2.0),
                CornerRadius::same(WIDGET_RADIUS + 2),
                ui.visuals().selection.stroke,
                egui::StrokeKind::Outside,
            );
        }
        // Centre the label, note and chip as one run.
        let mut x = rect.center().x - total / 2.0;
        let cy = rect.center().y;
        let at = |x: f32, g: &std::sync::Arc<egui::Galley>| egui::pos2(x, cy - g.size().y / 2.0);
        painter.galley(at(x, &label), label.clone(), fg);
        x += label.size().x;
        if let Some(n) = &note {
            x += gap;
            painter.galley(at(x, n), n.clone(), fg);
            x += n.size().x;
        }
        if let Some(c) = &chip {
            x += gap;
            let chip_rect = egui::Rect::from_min_size(
                egui::pos2(x, cy - (c.size().y + 6.0) / 2.0),
                egui::vec2(chip_w, c.size().y + 6.0),
            );
            let chip_fill = if accent {
                pal.accent.lerp_to_gamma(pal.on_accent, 0.18)
            } else {
                pal.row
            };
            painter.rect_filled(chip_rect, CornerRadius::same(5), dim(chip_fill));
            painter.galley(at(x + 6.0, c), c.clone(), fg);
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

/// A selectable toggle of fixed height (capture mode, preview mode).
pub fn toggle(ui: &mut egui::Ui, selected: bool, label: &str, width: f32) -> egui::Response {
    toggle_sized(ui, selected, label, egui::vec2(width, TOGGLE_HEIGHT))
}

/// A [`toggle`] of any size; `size.x` of 0 fits the label.
pub fn toggle_sized(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    size: egui::Vec2,
) -> egui::Response {
    let pal = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let fg = dim(if selected {
        pal.row_selected_text
    } else {
        pal.text
    });
    let family = if selected {
        strong_family()
    } else {
        FontFamily::Proportional
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), egui::FontId::new(15.0, family), fg);
    let width = if size.x > 0.0 {
        size.x
    } else {
        galley.size().x + 28.0
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, size.y), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, selected, label)
    });
    let (fill, stroke) = if selected {
        (pal.selected_tint, Stroke::new(1.5, pal.accent))
    } else if response.hovered() {
        (pal.row_hover, Stroke::new(1.0, pal.control_border))
    } else {
        (pal.row, Stroke::new(1.0, pal.control_border))
    };
    let radius = CornerRadius::same(WIDGET_RADIUS);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, dim(fill));
    painter.rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(WIDGET_RADIUS + 2),
            ui.visuals().selection.stroke,
            egui::StrokeKind::Outside,
        );
    }
    painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
    response
}

/// One segment of a segmented control: no outline of its own, the selected one gets the tint
/// and the accent text. Put segments in [`segmented`].
pub fn segment(ui: &mut egui::Ui, selected: bool, label: &str, size: egui::Vec2) -> egui::Response {
    let pal = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let fg = dim(if selected { pal.accent } else { pal.muted });
    let family = if selected {
        strong_family()
    } else {
        FontFamily::Proportional
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), egui::FontId::new(14.0, family), fg);
    let width = if size.x > 0.0 {
        size.x
    } else {
        galley.size().x + 24.0
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, size.y), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, selected, label)
    });
    let painter = ui.painter();
    let radius = CornerRadius::same(WIDGET_RADIUS - 2);
    if selected {
        painter.rect_filled(rect, radius, dim(pal.selected_tint));
    } else if response.hovered() {
        painter.rect_filled(rect, radius, dim(pal.row_hover));
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            radius,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let fg = if response.hovered() && !selected {
        dim(pal.text)
    } else {
        fg
    };
    painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
    response
}

/// The frame around [`segment`]s.
pub fn segmented<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let pal = palette(ui.ctx());
    egui::Frame::new()
        .fill(pal.background)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(WIDGET_RADIUS))
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                add(ui)
            })
            .inner
        })
        .inner
}

/// A row of tabs with an accent underline on the current one. Returns where each tab was drawn.
pub fn tabs<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    items: &[(T, &str)],
) -> Vec<egui::Rect> {
    let pal = palette(ui.ctx());
    let mut rects = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (value, label) in items {
            let selected = *current == *value;
            let (family, colour) = if selected {
                (strong_family(), pal.text)
            } else {
                (FontFamily::Proportional, pal.muted)
            };
            let galley = ui.painter().layout_no_wrap(
                (*label).to_owned(),
                egui::FontId::new(15.0, family),
                colour,
            );
            let size = egui::vec2(galley.size().x + 28.0, 40.0);
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            rects.push(rect);
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    true,
                    selected,
                    *label,
                )
            });
            let colour = if response.hovered() && !selected {
                pal.text
            } else {
                colour
            };
            let painter = ui.painter();
            painter.galley(rect.center() - galley.size() / 2.0, galley, colour);
            if selected {
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(rect.left() + 4.0, rect.bottom() - 2.0),
                        egui::pos2(rect.right() - 4.0, rect.bottom()),
                    ),
                    CornerRadius::same(1),
                    pal.accent,
                );
            }
            if response.has_focus() {
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(WIDGET_RADIUS),
                    ui.visuals().selection.stroke,
                    egui::StrokeKind::Inside,
                );
            }
            if response.clicked() && !selected {
                *current = *value;
            }
        }
    });
    rects
}

/// A rounded panel with the card fill. Rows inside are separated with [`card_row`].
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let pal = palette(ui.ctx());
    egui::Frame::new()
        .fill(pal.row)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(egui::Margin::symmetric(14, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui)
        })
}

/// One row of a [`card`]: `first` is false for the top row, every other row gets a divider.
pub fn card_row<R>(ui: &mut egui::Ui, first: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    card_row_sized(ui, first, 40.0, add)
}

/// A [`card_row`] with its own minimum height (the settings rows are roomier).
pub fn card_row_sized<R>(
    ui: &mut egui::Ui,
    first: bool,
    min_height: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    if !first {
        let pal = palette(ui.ctx());
        let y = ui.cursor().top();
        ui.painter()
            .hline(ui.max_rect().x_range(), y, Stroke::new(1.0, pal.border));
    }
    ui.horizontal(|ui| {
        ui.set_min_height(min_height);
        add(ui)
    })
    .inner
}

/// A checkbox with square corners; with the widget radius egui draws a circle at this size.
pub fn checkbox(
    ui: &mut egui::Ui,
    value: &mut bool,
    label: impl Into<egui::WidgetText>,
) -> egui::Response {
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
        ] {
            w.corner_radius = CornerRadius::same(4);
        }
        ui.checkbox(value, label)
    })
    .inner
}

/// A checkbox drawn as a full-width tile (the "show in export" grid). Checked tiles get the
/// accent outline and a tick, so the state is not carried by colour alone.
pub fn checkbox_tile(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
    let pal = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let fg = if *value {
        pal.row_selected_text
    } else {
        pal.text
    };
    let text = if *value {
        format!("✓ {label}") // i18n-ok
    } else {
        label.to_owned()
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text, egui::FontId::proportional(14.0), fg);
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let checked = *value;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, checked, label)
    });
    let (fill, stroke) = if checked {
        (pal.row_selected, Stroke::new(1.5, pal.accent))
    } else if response.hovered() {
        (pal.row_hover, Stroke::new(1.0, pal.control_border))
    } else {
        (pal.row, Stroke::new(1.0, pal.control_border))
    };
    let radius = CornerRadius::same(WIDGET_RADIUS);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, fill);
    painter.rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(WIDGET_RADIUS + 2),
            ui.visuals().selection.stroke,
            egui::StrokeKind::Outside,
        );
    }
    painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
    response
}

/// Small upper-case caption above a group, e.g. "CAPTURE MODE".
pub fn section_label(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let pal = palette(ui.ctx());
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(12.0)
            .family(strong_family())
            .color(pal.muted),
    )
}

/// A small monospace pill, e.g. for the capture backend.
pub fn chip(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let pal = palette(ui.ctx());
    egui::Frame::new()
        .fill(pal.row)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .monospace()
                    .size(12.0)
                    .color(pal.muted),
            )
        })
        .response
}

/// The app mark: an accent rounded square with a viewfinder ring.
pub fn logo(ui: &mut egui::Ui) {
    let pal = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(8), pal.accent);
    p.circle_stroke(rect.center(), 6.0, Stroke::new(2.0, pal.on_accent));
    p.circle_filled(rect.center(), 2.0, pal.on_accent);
}

/// WCAG relative luminance of an sRGB colour.
#[cfg(test)]
pub fn luminance(c: Color32) -> f64 {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG contrast ratio, 1.0 (identical) to 21.0 (black on white).
#[cfg(test)]
pub fn contrast(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_contrast_formula_matches_known_values() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-9);
        // #767676 on white is the classic "just passes AA" grey: 4.54:1.
        assert!(
            (contrast(Color32::from_rgb(0x76, 0x76, 0x76), Color32::WHITE) - 4.54).abs() < 0.02
        );
    }

    #[test]
    fn all_text_meets_the_wcag_aa_ratio_in_both_themes() {
        for p in [DARK, LIGHT] {
            let name = if p.dark { "dark" } else { "light" };
            let pairs = [
                ("text on panel", p.text, p.panel),
                ("text on background", p.text, p.background),
                ("text on rows", p.text, p.row),
                ("text on hovered rows", p.text, p.row_hover),
                ("muted text on panel", p.muted, p.panel),
                ("muted text on rows", p.muted, p.row),
                (
                    "muted text on selected rows",
                    p.row_selected_text,
                    p.row_selected,
                ),
                ("error text on panel", p.error, p.panel),
                ("error text on rows", p.error, p.row),
                ("error text on hovered rows", p.error, p.row_hover),
                ("accent text on panel", p.accent, p.panel),
                ("text on the accent fill", p.on_accent, p.accent),
                ("selected row text", p.row_selected_text, p.row_selected),
            ];
            for (what, fg, bg) in pairs {
                let c = contrast(fg, bg);
                assert!(
                    c >= 4.5,
                    "{name}: {what} has contrast {c:.2}:1, below 4.5:1"
                );
            }
        }
    }

    #[test]
    fn interface_parts_meet_the_three_to_one_ratio() {
        for p in [DARK, LIGHT] {
            let name = if p.dark { "dark" } else { "light" };
            for (what, fg, bg) in [
                ("saved indicator on panel", p.success, p.panel),
                ("focus ring on panel", p.focus, p.panel),
                ("focus ring on rows", p.focus, p.row),
                ("focus ring on the selected row", p.focus, p.row_selected),
                ("control border on panel", p.control_border, p.panel),
                ("control border on rows", p.control_border, p.row),
                ("accent outline on panel", p.accent, p.panel),
                ("selected row against the list", p.row_selected, p.panel),
            ] {
                // The selected row is told apart from its neighbours by more than colour (text weight is
                // unchanged, but it is filled and the preview outlines it too); require a clear difference.
                let need = if what.starts_with("selected row against") {
                    1.4
                } else {
                    3.0
                };
                let c = contrast(fg, bg);
                assert!(
                    c >= need,
                    "{name}: {what} has contrast {c:.2}:1, below {need}:1"
                );
            }
        }
    }

    #[test]
    fn the_themes_are_really_different_and_chosen_as_asked() {
        assert!(luminance(DARK.panel) < 0.1 && luminance(LIGHT.panel) > 0.8);
        assert!(use_dark(ThemeChoice::Dark, Some(egui::Theme::Light)));
        assert!(!use_dark(ThemeChoice::Light, Some(egui::Theme::Dark)));
        assert!(use_dark(ThemeChoice::System, Some(egui::Theme::Dark)));
        assert!(!use_dark(ThemeChoice::System, Some(egui::Theme::Light)));
        assert!(
            use_dark(ThemeChoice::System, None),
            "a desktop that does not say: dark"
        );
    }

    #[test]
    fn the_bold_face_is_only_named_once_the_fonts_are_installed() {
        assert_eq!(strong_family(), FontFamily::Proportional);
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        assert_eq!(strong_family(), FontFamily::Name("strong".into()));
        // Drawing with every family the interface uses must not panic.
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for family in [
                    FontFamily::Proportional,
                    FontFamily::Monospace,
                    strong_family(),
                ] {
                    ui.label(egui::RichText::new("Inter 0123").family(family));
                }
            });
        });
    }

    #[test]
    fn applying_a_theme_changes_egui_and_the_palette_follows() {
        let ctx = egui::Context::default();
        apply(&ctx, ThemeChoice::Light);
        assert!(!ctx.style().visuals.dark_mode);
        assert_eq!(palette(&ctx).panel, LIGHT.panel);
        assert_eq!(ctx.style().visuals.panel_fill, LIGHT.panel);
        apply(&ctx, ThemeChoice::Dark);
        assert!(ctx.style().visuals.dark_mode);
        assert_eq!(palette(&ctx).panel, DARK.panel);
        assert_eq!(
            ctx.style().visuals.selection.stroke.color,
            DARK.focus,
            "the focus ring uses the palette's focus colour"
        );
        assert!(ctx.style().visuals.selection.stroke.width >= 2.0);
    }
}
