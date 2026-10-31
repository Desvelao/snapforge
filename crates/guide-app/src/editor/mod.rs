//! The annotation editor window: tools, selection, move/resize, zoom and pan,
//! undo, save/reopen and export. Annotations are drawn live with egui; the
//! exported image is produced by `guide_render`.

mod paint;
pub(crate) mod props;
mod tools_ui;
mod view;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, CursorIcon, Key, PointerButton, Pos2, Sense, TextureHandle};
use guide_capture::Screenshot;
use guide_core::{
    Annotation, AnnotationId, Document, Editor, ItemId, Kind, ObscureMode, PixelRect, Point, RectF,
    Rgba, Style,
};
use guide_render::{measure_text, render, Image};
use guide_storage::{
    save_rgba, save_rgba_at, timestamped_stem, EditBundle, EditorOptions, ImageFormat,
    OriginalImage,
};

use crate::dialog::{self, Pick};
use crate::i18n::{tr, tr_fmt, tr_n};
use crate::profile_ui::{ProfilePicker, ProfileRequest};
use props::{from_color32, to_color32, Props};
use view::{resize_rect, Handle, View};

/// What the window around the editor must do after a frame.
pub enum Exit {
    Stay,
    Close,
}

/// Settings the editor takes from the main window.
pub struct Env {
    pub out_dir: PathBuf,
    pub format: ImageFormat,
    /// `snapforge gui`: Enter copies and quits, Ctrl+S saves an image file and quits.
    pub gui: bool,
}

/// `snapforge gui`: the shortcut list in the middle (selection screen) or the top right corner
/// (editor) of `free`, drawn over the
/// selection screen and the editor alike. It takes no input, so it never gets in the way of
/// a drag, and it stays away when `free` is too small to spare the room.
pub(crate) fn show_gui_help(ctx: &egui::Context, free: egui::Rect, centered: bool) {
    const MIN_FREE: egui::Vec2 = egui::vec2(560.0, 330.0);
    if free.width() < MIN_FREE.x || free.height() < MIN_FREE.y {
        return;
    }
    egui::Area::new(egui::Id::new("gui_help"))
        .order(egui::Order::Foreground)
        .pivot(if centered {
            egui::Align2::CENTER_CENTER
        } else {
            egui::Align2::RIGHT_TOP
        })
        .fixed_pos(if centered {
            free.center()
        } else {
            free.right_top() + egui::vec2(-12.0, 12.0)
        })
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(egui::Color32::from_black_alpha(190))
                .show(ui, |ui| {
                    let text = egui::Color32::WHITE;
                    egui::Grid::new(egui::Id::new("gui_help_rows"))
                        .num_columns(2)
                        .spacing([16.0, 3.0])
                        .show(ui, |ui| {
                            for (what, keys) in shortcut_rows() {
                                ui.label(egui::RichText::new(what).color(text));
                                ui.label(egui::RichText::new(keys).strong().color(text));
                                ui.end_row();
                            }
                        });
                });
        });
}

/// What `snapforge gui` offers on the keyboard, as (what it does, the keys), for the help
/// in the corner. Keep it in step with `Editor::handle_keys`.
pub(crate) fn shortcut_rows() -> Vec<(String, String)> {
    let tools: Vec<&str> = Tool::ALL.iter().map(|t| t.key().name()).collect();
    vec![
        (tr("Select a region").into(), tr("Drag").into()),
        (tr("Copy to clipboard").into(), tr("Enter / Ctrl+C").into()),
        (
            tr("Save the screenshot in a file").into(),
            tr("Ctrl+S").into(),
        ),
        (tr("Cancel").into(), tr("Esc").into()),
        (tr("Undo / redo").into(), tr("Ctrl+Z / Ctrl+Y").into()),
        (
            tr("Delete the selected annotation").into(),
            tr("Del").into(),
        ),
        (tr("Fit / actual size").into(), tr("0 / 1").into()),
        (tr("Choose a tool").into(), tools.join(" ")),
        (tr("Show or hide this help").into(), tr("F1").into()),
    ]
}

#[derive(Clone, Copy)]
enum Finish {
    Copy,
    SaveFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Pen,
    Highlighter,
    Line,
    Arrow,
    Rectangle,
    Ellipse,
    Text,
    Marker,
    Blur,
    Crop,
    Eraser,
}

impl Tool {
    const ALL: [Tool; 12] = [
        Tool::Select,
        Tool::Pen,
        Tool::Highlighter,
        Tool::Line,
        Tool::Arrow,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Text,
        Tool::Marker,
        Tool::Blur,
        Tool::Crop,
        Tool::Eraser,
    ];

    fn label(self) -> &'static str {
        match self {
            Tool::Select => tr("Select"),
            Tool::Pen => tr("Pen"),
            Tool::Highlighter => tr("Highlighter"),
            Tool::Line => tr("Line"),
            Tool::Arrow => tr("Arrow"),
            Tool::Rectangle => tr("Rectangle"),
            Tool::Ellipse => tr("Ellipse"),
            Tool::Text => tr("Text"),
            Tool::Marker => tr("Marker"),
            Tool::Blur => tr("Blur"),
            Tool::Crop => tr("Crop"),
            Tool::Eraser => tr("Eraser"),
        }
    }

    fn key(self) -> Key {
        match self {
            Tool::Select => Key::V,
            Tool::Pen => Key::P,
            Tool::Highlighter => Key::H,
            Tool::Line => Key::L,
            Tool::Arrow => Key::A,
            Tool::Rectangle => Key::R,
            Tool::Ellipse => Key::E,
            Tool::Text => Key::T,
            Tool::Marker => Key::M,
            Tool::Blur => Key::B,
            Tool::Crop => Key::C,
            Tool::Eraser => Key::X,
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Tool::Select => tr("Click to select, drag to move, drag the handles to resize"),
            Tool::Pen => tr("Drag to draw freehand"),
            Tool::Highlighter => tr("Drag to highlight"),
            Tool::Line => tr("Drag to draw a line"),
            Tool::Arrow => tr("Drag from the tail to the tip"),
            Tool::Rectangle => tr("Drag to draw a rectangle"),
            Tool::Ellipse => tr("Drag to draw an ellipse"),
            Tool::Text => tr("Click where the text goes"),
            Tool::Marker => tr("Click to place the next numbered marker"),
            Tool::Blur => tr("Drag over sensitive content to blur or pixelate it"),
            Tool::Crop => tr("Drag the area to keep; the original is not changed"),
            Tool::Eraser => tr("Click an annotation to remove it"),
        }
    }
}

enum Gesture {
    /// Freehand tools fill `points`; shape tools use `start` and `cur`.
    Draw {
        tool: Tool,
        start: Point,
        cur: Point,
        points: Vec<Point>,
    },
    Move {
        original: Annotation,
        start: Point,
        current: Annotation,
    },
    Resize {
        original: Annotation,
        handle: Handle,
        bounds: RectF,
        current: Annotation,
    },
    /// Dragging a handle of the existing crop.
    CropResize {
        original: RectF,
        handle: Handle,
        current: RectF,
    },
    Pan,
    Erase,
}

/// The part of a document that is saved: its annotations and crop (not the id counter, which
/// undo does not wind back).
#[derive(Clone, PartialEq)]
struct SavedState {
    annotations: Vec<Annotation>,
    crop: Option<PixelRect>,
}

impl SavedState {
    fn of(doc: &Document) -> Self {
        Self {
            annotations: doc.annotations.clone(),
            crop: doc.crop,
        }
    }
}

struct TextEditing {
    at: Point,
    text: String,
    /// `Some` when editing an existing text annotation.
    id: Option<AnnotationId>,
    fresh: bool,
}

/// A rendered preview and where it sits, or why rendering failed.
type PreviewResult = Result<(Image, Point), String>;

#[derive(Default)]
struct Preview {
    on: bool,
    /// The document generation `tex` was rendered from.
    rendered_gen: Option<u64>,
    tex: Option<TextureHandle>,
    /// Where the (possibly cropped) preview sits in image coordinates.
    offset: Point,
    pending: Option<(u64, mpsc::Receiver<PreviewResult>)>,
}

pub struct EditorView {
    editor: Editor,
    original: Arc<Image>,
    tex: TextureHandle,
    size: (u32, u32),
    view: View,
    needs_fit: bool,
    tool: Tool,
    props: Props,
    selected: Option<AnnotationId>,
    gesture: Option<Gesture>,
    text: Option<TextEditing>,
    /// Pixelated/blurred pictures of `Obscure` annotations, keyed by id.
    patches: HashMap<AnnotationId, (Kind, TextureHandle, RectF)>,
    preview: Preview,
    export_rx: Option<mpsc::Receiver<String>>,
    /// Result of a `snapforge gui` finishing action running in the background.
    /// `Ok(false)`: the user cancelled the save dialog, so the editor stays open.
    finish_rx: Option<mpsc::Receiver<Result<bool, String>>>,
    finished: bool,
    /// `snapforge gui`: the shortcut help in the corner was turned off with F1.
    help_hidden: bool,
    /// While set, the fit is redone whenever the canvas size changes (the window is still
    /// being resized from the fullscreen overlay) until the user zooms or pans.
    refit_on_resize: bool,
    auto_view: Option<View>,
    last_canvas_size: Option<egui::Vec2>,
    failure: Option<String>,
    bundle: Option<EditBundle>,
    stem: String,
    /// Set when editing one step of a guide: saving then hands the annotations back to the guide.
    step: Option<ItemId>,
    commit: Option<Document>,
    /// Ctrl+W was pressed; acted on in `show`.
    close_key: bool,
    /// Bumped on every change to the document (including undo and redo).
    generation: u64,
    /// Widths of the top bar's three groups in the last frame, to centre the middle one.
    bar_widths: [f32; 3],
    /// What the document looked like when it was last saved (or opened); the edit is unsaved
    /// only while the document differs from it, so undoing back to it is not a change.
    saved: SavedState,
    status: String,
    confirm_close: bool,
    /// The unsaved-changes dialog came from closing the whole window, so quit once resolved.
    quit_after: bool,
    /// Where the width slider was drawn last (window-relative points); test hook.
    width_slider: Option<egui::Rect>,
    /// Tool settings as the user last chose them, kept in the active profile. Unlike `props`,
    /// selecting an annotation does not change them.
    profile_props: Props,
    profile_names: Vec<String>,
    profile_active: String,
    profile_picker: ProfilePicker,
    profile_request: Option<ProfileRequest>,
    prop_touch: Option<Instant>,
    report_path: Option<PathBuf>,
    last_report: String,
}

// ---- pure helpers (unit-tested below) ------------------------------------

/// Bounds of the shape itself; text uses its exact measured size.
fn exact_bounds(a: &Annotation) -> RectF {
    match &a.kind {
        Kind::Text { at, text, size, .. } => {
            let (w, h) = measure_text(text, *size);
            RectF {
                x: at.x,
                y: at.y,
                w,
                h,
            }
        }
        _ => a.geometry_bounds(),
    }
}

fn hit(a: &Annotation, p: Point, tol: f32) -> bool {
    match &a.kind {
        Kind::Text { .. } => exact_bounds(a).inflate(tol).contains(p),
        _ => a.hit_test(p, tol),
    }
}

/// The annotation drawn on top at `p`.
fn topmost_at(doc: &Document, p: Point, tol: f32) -> Option<AnnotationId> {
    doc.annotations
        .iter()
        .rev()
        .find(|a| hit(a, p, tol))
        .map(|a| a.id)
}

/// Whole-pixel area of `rect` inside an image, or `None` if nothing is inside.
fn resize_cursor(h: Handle) -> CursorIcon {
    match h {
        Handle::NW | Handle::SE => CursorIcon::ResizeNwSe,
        Handle::NE | Handle::SW => CursorIcon::ResizeNeSw,
        Handle::N | Handle::S => CursorIcon::ResizeVertical,
        Handle::E | Handle::W => CursorIcon::ResizeHorizontal,
    }
}

fn clipped_pixels(rect: RectF, size: (u32, u32)) -> Option<PixelRect> {
    let x0 = rect.x.floor().max(0.0) as u32;
    let y0 = rect.y.floor().max(0.0) as u32;
    let x1 = (rect.right().ceil().max(0.0) as u32).min(size.0);
    let y1 = (rect.bottom().ceil().max(0.0) as u32).min(size.1);
    (x1 > x0 && y1 > y0).then(|| PixelRect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}

/// The pixelated/blurred picture for an `Obscure` annotation, computed from the
/// original pixels under it (annotations below it are not included here; the
/// export preview shows the exact result). Returns the picture and where it goes.
fn obscure_patch(original: &Image, a: &Annotation) -> Option<(Image, RectF)> {
    let Kind::Obscure {
        rect,
        mode,
        strength,
    } = &a.kind
    else {
        return None;
    };
    let area = clipped_pixels(*rect, (original.width, original.height))?;
    let mut rgba = Vec::with_capacity(area.w as usize * area.h as usize * 4);
    for row in area.y..area.y + area.h {
        let start = (row as usize * original.width as usize + area.x as usize) * 4;
        rgba.extend_from_slice(&original.rgba[start..start + area.w as usize * 4]);
    }
    let small = Image {
        width: area.w,
        height: area.h,
        rgba,
    };
    let mut doc = Document::default();
    let mut ed = Editor::new(std::mem::take(&mut doc));
    ed.add(
        Kind::Obscure {
            rect: RectF {
                x: 0.0,
                y: 0.0,
                w: area.w as f32,
                h: area.h as f32,
            },
            mode: *mode,
            strength: *strength,
        },
        Style::default(),
    );
    let out = render(&small, ed.document()).ok()?;
    Some((
        out,
        RectF {
            x: area.x as f32,
            y: area.y as f32,
            w: area.w as f32,
            h: area.h as f32,
        },
    ))
}

/// Builds the annotation a drag would create, or `None` if it is too small to keep.
fn drafted(
    tool: Tool,
    start: Point,
    cur: Point,
    points: &[Point],
    props: &Props,
) -> Option<Annotation> {
    let far = |a: Point, b: Point| (a.x - b.x).abs() >= 2.0 || (a.y - b.y).abs() >= 2.0;
    let (kind, style) = match tool {
        Tool::Pen => (
            Kind::Pen {
                points: points.to_vec(),
            },
            props.style_for(false),
        ),
        Tool::Highlighter => (
            Kind::Highlighter {
                points: points.to_vec(),
            },
            props.style_for(true),
        ),
        Tool::Line if far(start, cur) => (
            Kind::Line {
                from: start,
                to: cur,
            },
            props.style_for(false),
        ),
        Tool::Arrow if far(start, cur) => (
            Kind::Arrow {
                from: start,
                to: cur,
            },
            props.style_for(false),
        ),
        Tool::Rectangle if far(start, cur) => (
            Kind::Rectangle {
                rect: RectF::from_points(start, cur),
            },
            props.style_for(false),
        ),
        Tool::Ellipse if far(start, cur) => (
            Kind::Ellipse {
                rect: RectF::from_points(start, cur),
            },
            props.style_for(false),
        ),
        Tool::Blur if far(start, cur) => (
            Kind::Obscure {
                rect: RectF::from_points(start, cur),
                mode: props.obscure_mode,
                strength: props.obscure_strength,
            },
            Style::default(),
        ),
        _ => return None,
    };
    Some(Annotation {
        id: AnnotationId(0),
        kind,
        style,
    })
}

fn texture_from(ctx: &egui::Context, name: &str, img: Image) -> TextureHandle {
    crate::app::texture_for(
        ctx,
        name,
        &Screenshot {
            width: img.width,
            height: img.height,
            rgba: img.rgba,
        },
    )
}

impl EditorView {
    pub fn new(
        ctx: &egui::Context,
        shot: &Screenshot,
        document: Document,
        bundle: Option<EditBundle>,
        stem: String,
    ) -> Self {
        let original = Arc::new(Image {
            width: shot.width,
            height: shot.height,
            rgba: shot.rgba.clone(),
        });
        Self {
            tex: crate::app::texture_for(ctx, "editor-original", shot),
            size: (shot.width, shot.height),
            saved: SavedState::of(&document),
            editor: Editor::new(document),
            original,
            view: View {
                origin: egui::pos2(0.0, 0.0),
                zoom: 1.0,
            },
            needs_fit: true,
            tool: Tool::Select,
            props: Props::default(),
            selected: None,
            gesture: None,
            text: None,
            patches: HashMap::new(),
            preview: Preview::default(),
            export_rx: None,
            finish_rx: None,
            finished: false,
            help_hidden: false,
            refit_on_resize: false,
            auto_view: None,
            last_canvas_size: None,
            failure: None,
            bundle,
            stem,
            step: None,
            commit: None,
            close_key: false,
            generation: 0,
            bar_widths: [0.0; 3],
            status: String::new(),
            confirm_close: false,
            quit_after: false,
            width_slider: None,
            profile_props: Props::default(),
            profile_names: Vec::new(),
            profile_active: String::new(),
            profile_picker: ProfilePicker::default(),
            profile_request: None,
            prop_touch: None,
            report_path: std::env::var_os("SNAPFORGE_TEST_EDITOR_REPORT").map(PathBuf::from),
            last_report: String::new(),
        }
    }

    /// Edits one step of a guide. Saving (Ctrl+S, Done) hands the annotations back
    /// through [`EditorView::take_commit`]; nothing is written to disk by the editor itself.
    pub fn for_step(
        ctx: &egui::Context,
        shot: &Screenshot,
        document: Document,
        step: ItemId,
    ) -> Self {
        let mut v = Self::new(ctx, shot, document, None, format!("step-{}", step.0));
        v.step = Some(step);
        v
    }

    /// The annotations saved since the last call, with the step they belong to.
    pub fn take_commit(&mut self) -> Option<(ItemId, Document)> {
        let step = self.step?;
        self.commit.take().map(|d| (step, d))
    }

    /// The edit folder this edit is saved in, once it has been saved.
    pub fn saved_dir(&self) -> Option<&std::path::Path> {
        self.bundle.as_ref().map(|b| b.dir())
    }

    pub fn from_bundle(
        ctx: &egui::Context,
        bundle: EditBundle,
        original: OriginalImage,
        document: Document,
    ) -> Self {
        let stem = bundle
            .dir()
            .file_stem()
            .map_or_else(|| "edit".to_string(), |s| s.to_string_lossy().into_owned());
        let shot = Screenshot {
            width: original.width,
            height: original.height,
            rgba: original.rgba,
        };
        Self::new(ctx, &shot, document, Some(bundle), stem)
    }

    /// The user asked to close the whole window. Returns `true` if closing must be
    /// cancelled because there are unsaved changes (a dialog then asks what to do).
    pub fn on_window_close_requested(&mut self) -> bool {
        self.commit_text();
        if self.is_dirty() {
            self.confirm_close = true;
            self.quit_after = true;
            true
        } else {
            false
        }
    }

    /// After `Exit::Close`: whether the whole application should quit too.
    pub fn quit_requested(&self) -> bool {
        self.quit_after
    }

    pub fn is_dirty(&self) -> bool {
        let doc = self.editor.document();
        doc.annotations != self.saved.annotations || doc.crop != self.saved.crop
    }

    fn touched(&mut self, changed: bool) {
        if changed {
            self.generation += 1;
        }
    }

    fn selected_annotation(&self) -> Option<Annotation> {
        self.selected
            .and_then(|id| self.editor.document().find(id))
            .cloned()
    }

    fn select(&mut self, id: Option<AnnotationId>) {
        self.selected = id;
        if let Some(a) = self.selected_annotation() {
            self.props.load(&a);
        }
    }

    fn set_tool(&mut self, tool: Tool) {
        self.commit_text();
        self.gesture = None;
        self.tool = tool;
        if tool != Tool::Select {
            self.selected = None;
        }
    }

    // ---- document operations ---------------------------------------------

    fn add(&mut self, kind: Kind, style: Style) -> AnnotationId {
        let id = self.editor.add(kind, style);
        self.touched(true);
        id
    }

    fn remove(&mut self, id: AnnotationId) {
        let changed = self.editor.remove(id);
        self.patches.remove(&id);
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.touched(changed);
    }

    fn replace(&mut self, a: Annotation) {
        self.patches.remove(&a.id);
        let changed = self.editor.replace(a);
        self.touched(changed);
    }

    fn undo(&mut self) {
        self.commit_text();
        let changed = self.editor.undo();
        self.after_history(changed);
    }

    fn redo(&mut self) {
        self.commit_text();
        let changed = self.editor.redo();
        self.after_history(changed);
    }

    /// Deletes the selected annotation, after finishing a text that is being typed.
    fn delete_selected(&mut self) {
        self.commit_text();
        if let Some(id) = self.selected {
            self.remove(id);
        }
    }

    fn after_history(&mut self, changed: bool) {
        self.touched(changed);
        self.patches.clear();
        self.gesture = None;
        if self
            .selected
            .is_some_and(|id| self.editor.document().find(id).is_none())
        {
            self.selected = None;
        }
        // The controls must show the restored values, not what was last dragged.
        if let Some(a) = self.selected_annotation() {
            self.props.load(&a);
        }
    }

    fn set_crop(&mut self, crop: Option<PixelRect>) {
        let changed = self.editor.set_crop(crop);
        self.touched(changed);
    }

    // ---- text ---------------------------------------------------------------

    fn start_text(&mut self, at: Point, text: String, id: Option<AnnotationId>) {
        self.commit_text();
        self.text = Some(TextEditing {
            at,
            text,
            id,
            fresh: true,
        });
    }

    fn commit_text(&mut self) {
        let Some(t) = self.text.take() else { return };
        if t.text.trim().is_empty() {
            if let Some(id) = t.id {
                self.remove(id); // emptying a text removes it
            }
            return;
        }
        match t.id.and_then(|id| self.editor.document().find(id).cloned()) {
            Some(mut a) => {
                if let Kind::Text { text, .. } = &mut a.kind {
                    *text = t.text;
                }
                self.replace(a);
            }
            None => {
                let kind = Kind::Text {
                    at: t.at,
                    text: t.text,
                    size: self.props.text_size,
                    bold: self.props.bold,
                };
                let id = self.add(kind, self.props.style_for(false));
                self.select(Some(id));
                self.tool = Tool::Select;
            }
        }
    }

    // ---- background work -------------------------------------------------------

    fn poll_async(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.finish_rx {
            if let Ok(result) = rx.try_recv() {
                self.finish_rx = None;
                match result {
                    Ok(true) => self.finished = true,
                    Ok(false) => self.status = tr("Save cancelled").into(),
                    Err(e) => {
                        self.status = tr_fmt("Save failed: {error}", &[("error", &e)]);
                        self.failure = Some(e);
                    }
                }
            }
        }
        if let Some(rx) = &self.export_rx {
            if let Ok(msg) = rx.try_recv() {
                self.status = msg;
                self.export_rx = None;
            }
        }
        if let Some((generation, rx)) = &self.preview.pending {
            match rx.try_recv() {
                Ok(Ok((img, offset))) => {
                    self.preview.tex = Some(texture_from(ctx, "editor-preview", img));
                    self.preview.offset = offset;
                    self.preview.rendered_gen = Some(*generation);
                    self.preview.pending = None;
                }
                Ok(Err(e)) => {
                    self.status = tr_fmt("Preview failed: {error}", &[("error", &e)]);
                    self.preview.on = false;
                    self.preview.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.preview.pending = None,
            }
        }
        if self.preview.on
            && self.preview.pending.is_none()
            && self.preview.rendered_gen != Some(self.generation)
        {
            let (orig, doc, generation, ctx2) = (
                self.original.clone(),
                self.editor.document().clone(),
                self.generation,
                ctx.clone(),
            );
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let offset = doc
                    .crop
                    .map_or(Point::new(0.0, 0.0), |c| Point::new(c.x as f32, c.y as f32));
                let _ = tx.send(
                    render(&orig, &doc)
                        .map(|i| (i, offset))
                        .map_err(|e| e.to_string()),
                );
                ctx2.request_repaint();
            });
            self.preview.pending = Some((generation, rx));
        }
    }

    fn save(&mut self, env: &Env) {
        if self.step.is_some() {
            self.commit = Some(self.editor.document().clone());
            self.saved = SavedState::of(self.editor.document());
            self.status = tr("Saved to the guide").into();
            return;
        }
        let result = (|| -> Result<PathBuf, String> {
            if self.bundle.is_none() {
                let mut n = 1;
                let dir = loop {
                    let name = if n == 1 {
                        format!("{}.{}", self.stem, guide_core::EDIT_EXTENSION)
                    } else {
                        format!("{}-{n}.{}", self.stem, guide_core::EDIT_EXTENSION)
                    };
                    let dir = env.out_dir.join(name);
                    if !dir.exists() {
                        break dir;
                    }
                    n += 1;
                };
                let o = &self.original;
                self.bundle = Some(
                    EditBundle::create(&dir, o.width, o.height, &o.rgba)
                        .map_err(|e| e.to_string())?,
                );
            }
            let bundle = self.bundle.as_ref().expect("bundle was just created");
            bundle
                .save_document(self.editor.document())
                .map_err(|e| e.to_string())?;
            Ok(bundle.dir().to_path_buf())
        })();
        self.status = match result {
            Ok(dir) => {
                self.saved = SavedState::of(self.editor.document());
                tr_fmt("Saved edit to {folder}", &[("folder", &dir.display())])
            }
            Err(e) => tr_fmt("Save failed: {error}", &[("error", &e)]),
        };
    }

    /// For `snapforge gui`: keep the image fitted while the window settles after the overlay.
    pub fn refit_while_settling(&mut self) {
        self.refit_on_resize = true;
    }

    /// A failure of a finishing action that the window around the editor should show once.
    pub fn take_failure(&mut self) -> Option<String> {
        self.failure.take()
    }

    /// `snapforge gui`: renders the annotated image, copies or saves it, then the editor closes.
    fn finish(&mut self, ctx: &egui::Context, env: &Env, how: Finish) {
        if self.finish_rx.is_some() || self.finished {
            return;
        }
        self.commit_text();
        let (orig, doc, ctx2) = (
            self.original.clone(),
            self.editor.document().clone(),
            ctx.clone(),
        );
        let (dir, format) = (env.out_dir.clone(), env.format);
        let name = format!("{}.{}", timestamped_stem(), format.extension());
        let title = tr("Save the screenshot").to_string();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result: Result<bool, String> = (|| {
                // The system dialog blocks this worker, not the window.
                let chosen = match how {
                    Finish::Copy => None,
                    Finish::SaveFile => match dialog::save_file(&dir, &name, &title) {
                        Pick::Chosen(path) => Some(path),
                        Pick::Cancelled => return Ok(false),
                        // No dialog on this system: keep the old behaviour.
                        Pick::Unavailable => None,
                    },
                };
                let img = render(&orig, &doc).map_err(|e| e.to_string())?;
                match (how, chosen) {
                    (Finish::Copy, _) => crate::clipboard::copy_image_persistent(&img),
                    (Finish::SaveFile, Some(path)) => {
                        save_rgba_at(&path, format, img.width, img.height, &img.rgba)
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    }
                    (Finish::SaveFile, None) => save_rgba(
                        &dir,
                        &timestamped_stem(),
                        format,
                        img.width,
                        img.height,
                        &img.rgba,
                    )
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                }
                .map(|()| true)
            })();
            let _ = tx.send(result);
            ctx2.request_repaint();
        });
        self.finish_rx = Some(rx);
        self.status = tr("Working...").into();
    }

    fn export(&mut self, ctx: &egui::Context, env: &Env) {
        if self.export_rx.is_some() {
            return;
        }
        let (orig, doc, ctx2) = (
            self.original.clone(),
            self.editor.document().clone(),
            ctx.clone(),
        );
        let (dir, format, stem) = (
            env.out_dir.clone(),
            env.format,
            format!("{}-edited", timestamped_stem()),
        );
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let msg = match render(&orig, &doc) {
                Ok(img) => match save_rgba(&dir, &stem, format, img.width, img.height, &img.rgba) {
                    Ok(p) => tr_fmt(
                        "Exported {width}x{height} to {path}",
                        &[
                            ("width", &img.width),
                            ("height", &img.height),
                            ("path", &p.display()),
                        ],
                    ),
                    Err(e) => tr_fmt("Export failed: {error}", &[("error", &e)]),
                },
                Err(e) => tr_fmt("Export failed: {error}", &[("error", &e)]),
            };
            let _ = tx.send(msg);
            ctx2.request_repaint();
        });
        self.export_rx = Some(rx);
        self.status = tr("Exporting...").into();
    }

    // ---- frame ---------------------------------------------------------------------

    pub fn show(&mut self, ctx: &egui::Context, env: &Env) -> Exit {
        self.poll_async(ctx);
        self.handle_keys(ctx, env);
        let mut exit = Exit::Stay;
        if std::mem::take(&mut self.close_key) {
            if self.step.is_some() {
                self.save(env); // "Done": keep the changes
                exit = Exit::Close;
            } else {
                self.request_close(&mut exit);
            }
        }

        egui::TopBottomPanel::top("editor_tools").show(ctx, |ui| {
            ui.add_space(6.0);
            self.top_bar(ui, ctx, env, &mut exit);
            ui.add_space(6.0);
        });
        if env.gui {
            egui::TopBottomPanel::bottom("editor_gui_bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let idle = self.finish_rx.is_none();
                    if ui
                        .add_enabled(idle, egui::Button::new(tr("Copy (Enter)")))
                        .clicked()
                    {
                        self.finish(ctx, env, Finish::Copy);
                    }
                    if ui
                        .add_enabled(idle, egui::Button::new(tr("Save as file (Ctrl+S)")))
                        .clicked()
                    {
                        self.finish(ctx, env, Finish::SaveFile);
                    }
                    if ui.button(tr("Cancel (Esc)")).clicked() {
                        self.request_close(&mut exit);
                    }
                });
            });
        }
        let pal = crate::theme::palette(ctx);
        egui::TopBottomPanel::bottom("editor_status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let summary = tr_n(
                    "{width}x{height}  zoom {zoom}%  {n} annotation",
                    "{width}x{height}  zoom {zoom}%  {n} annotations",
                    self.editor.document().annotations.len(),
                    &[
                        ("width", &self.size.0),
                        ("height", &self.size.1),
                        (
                            "zoom",
                            &format!("{:.0}", self.view.zoom * ctx.pixels_per_point() * 100.0),
                        ),
                    ],
                );
                let summary = if self.is_dirty() {
                    tr_fmt("{summary} - unsaved changes", &[("summary", &summary)])
                } else {
                    summary
                };
                ui.label(egui::RichText::new(summary).monospace().color(pal.muted));
                if !self.status.is_empty() {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(&self.status)
                            .monospace()
                            .color(pal.muted),
                    );
                }
            });
        });
        let side = egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::same(12));
        egui::SidePanel::left("editor_tool_palette")
            .exact_width(168.0)
            .resizable(false)
            .frame(side)
            .show(ctx, |ui| self.tool_palette(ui));
        egui::SidePanel::right("editor_properties")
            .exact_width(288.0)
            .resizable(false)
            .frame(side)
            .show(ctx, |ui| self.props_panel(ui, ctx));
        if env.gui {
            self.gui_help_overlay(ctx);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.canvas(ui, ctx));

        self.text_popup(ctx);
        if self.confirm_close && !self.is_dirty() {
            // Saved (for example with Ctrl+S) while the dialog was open: nothing left to ask.
            self.confirm_close = false;
            if self.quit_after {
                exit = Exit::Close;
            }
        }
        if self.confirm_close {
            egui::Window::new(tr("Unsaved changes"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(tr("This edit has changes that are not saved."));
                    ui.horizontal(|ui| {
                        if !env.gui && ui.button(tr("Save and close")).clicked() {
                            self.save(env);
                            if !self.is_dirty() {
                                exit = Exit::Close;
                            }
                            self.confirm_close = false;
                        }
                        if ui.button(tr("Discard changes")).clicked() {
                            exit = Exit::Close;
                        }
                        if ui.button(tr("Keep editing")).clicked() {
                            self.confirm_close = false;
                            self.quit_after = false;
                        }
                    });
                });
        }
        if self.finished {
            exit = Exit::Close;
        }
        self.write_report(ctx);
        exit
    }

    /// `snapforge gui`: the shortcuts in a corner of the canvas, out of the way while the
    /// mouse is drawing or the image is too small to spare the room.
    fn gui_help_overlay(&self, ctx: &egui::Context) {
        let mouse_busy = ctx.input(|i| i.pointer.any_down());
        if self.help_hidden || mouse_busy || self.gesture.is_some() || self.text.is_some() {
            return;
        }
        let free = ctx.available_rect();
        // A small picture is not covered by the list (about 560 x 340 points); one that fills the
        // canvas cannot avoid it, so the list stays in the corner on top of it.
        let help = egui::Rect::from_min_size(
            egui::pos2(free.right() - 12.0 - 560.0, free.top() + 12.0),
            egui::vec2(560.0, 340.0),
        );
        let image = self.view.screen_rect(RectF {
            x: 0.0,
            y: 0.0,
            w: self.size.0 as f32,
            h: self.size.1 as f32,
        });
        if help.intersects(image) && image.width() < free.width() * 0.6 {
            return;
        }
        show_gui_help(ctx, free, false);
    }

    /// Takes the tool settings of the active profile.
    pub fn set_options(&mut self, options: &EditorOptions) {
        self.props = Props::from(options);
        self.profile_props = self.props.clone();
    }

    /// The tool settings to keep in the active profile.
    pub fn options(&self) -> EditorOptions {
        EditorOptions::from(&self.profile_props)
    }

    /// The profiles to offer and the selected one; call every frame before `show`.
    pub fn set_profiles(&mut self, names: &[String], active: &str) {
        if self.profile_active != active || self.profile_names != names {
            self.profile_names = names.to_vec();
            self.profile_active = active.to_string();
        }
    }

    /// A profile change the user asked for since the last call.
    pub fn take_profile_request(&mut self) -> Option<ProfileRequest> {
        self.profile_request.take()
    }

    /// Starts with the help turned off or on, as chosen on the selection screen.
    pub fn set_help_hidden(&mut self, hidden: bool) {
        self.help_hidden = hidden;
    }

    fn request_close(&mut self, exit: &mut Exit) {
        self.commit_text();
        if self.is_dirty() {
            self.confirm_close = true;
        } else {
            *exit = Exit::Close;
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context, env: &Env) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let (cmd, shift, esc, del) = ctx.input(|i| {
            (
                i.modifiers.command,
                i.modifiers.shift,
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
            )
        });
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        if env.gui && pressed(Key::F1) {
            self.help_hidden = !self.help_hidden;
        }
        // The export preview is read-only, like the mouse (see `interact`): keys that edit wait.
        let editing = !self.preview.on;
        if cmd {
            if editing {
                if pressed(Key::Z) && shift || pressed(Key::Y) {
                    self.redo();
                } else if pressed(Key::Z) {
                    self.undo();
                }
            }
            if pressed(Key::S) {
                if env.gui {
                    self.finish(ctx, env, Finish::SaveFile);
                } else {
                    self.save(env);
                }
            }
            if env.gui && pressed(Key::C) {
                self.finish(ctx, env, Finish::Copy);
            }
            if pressed(Key::E) {
                self.export(ctx, env);
            }
            if pressed(Key::W) {
                self.close_key = true;
            }
            return;
        }
        if env.gui && pressed(Key::Enter) {
            self.finish(ctx, env, Finish::Copy);
        }
        if esc && self.gesture.take().is_none() && self.selected.take().is_none() {
            // In gui mode a second Escape (nothing left to cancel) leaves the capture.
            if env.gui && self.tool == Tool::Select {
                self.close_key = true;
            }
            self.tool = Tool::Select;
        }
        if del && editing {
            self.delete_selected();
        }
        if pressed(Key::Num0) {
            self.needs_fit = true;
        }
        if pressed(Key::Num1) {
            self.view.zoom = 1.0 / ctx.pixels_per_point();
        }
        for t in Tool::ALL {
            if editing && pressed(t.key()) && self.tool != t {
                self.set_tool(t);
            }
        }
    }

    /// The bar across the top: edit actions on the left, view controls in the middle, saving and
    /// leaving on the right. The groups' widths of the last frame are used to centre the middle one.
    fn top_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, env: &Env, exit: &mut Exit) {
        let [lw, cw, rw] = self.bar_widths;
        // Each group and each gap is an item, so four spacings sit between the five of them.
        const SPACING: f32 = 6.0;
        let gap = ((ui.available_width() - lw - cw - rw - 4.0 * SPACING) / 2.0).max(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING;
            let l = ui.horizontal(|ui| self.bar_left(ui)).response.rect.width();
            ui.add_space(gap);
            let c = ui
                .horizontal(|ui| self.bar_centre(ui, ctx))
                .response
                .rect
                .width();
            ui.add_space(gap);
            let r = ui
                .horizontal(|ui| self.bar_right(ui, ctx, env, exit))
                .response
                .rect
                .width();
            if [l, c, r]
                .iter()
                .zip(self.bar_widths)
                .any(|(new, old)| (new - old).abs() > 0.5)
            {
                self.bar_widths = [l, c, r];
                ctx.request_repaint();
            }
        });
    }

    fn bar_left(&mut self, ui: &mut egui::Ui) {
        use crate::theme::{Btn, Emphasis};
        let quiet = |ui: &mut egui::Ui, label: &str, enabled: bool, hover: &str| {
            ui.add_enabled_ui(enabled, |ui| {
                Btn::new(label, Emphasis::Secondary)
                    .compact()
                    .show(ui)
                    .on_hover_text(hover)
                    .clicked()
            })
            .inner
        };
        if quiet(ui, tr("Undo"), self.editor.can_undo(), tr("Ctrl+Z")) {
            self.undo();
        }
        if quiet(
            ui,
            tr("Redo"),
            self.editor.can_redo(),
            tr("Ctrl+Y or Ctrl+Shift+Z"),
        ) {
            self.redo();
        }
        if quiet(ui, tr("Delete"), self.selected.is_some(), tr("Delete key")) {
            self.delete_selected();
        }
        if quiet(
            ui,
            tr("Clear crop"),
            self.editor.document().crop.is_some(),
            tr("Show the whole picture again"),
        ) {
            self.set_crop(None);
        }
    }

    fn bar_centre(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let ppp = ctx.pixels_per_point();
        let fit = self.auto_view == Some(self.view);
        let actual = !fit && (self.view.zoom * ppp - 1.0).abs() < 0.005;
        crate::theme::segmented(ui, |ui| {
            let size = egui::vec2(0.0, 26.0);
            if crate::theme::segment(ui, fit, tr("Fit"), size)
                .on_hover_text(tr("Fit (0)"))
                .clicked()
            {
                self.needs_fit = true;
            }
            if crate::theme::segment(ui, actual, tr("100%"), size)
                .on_hover_text(tr("100% (1)"))
                .clicked()
            {
                self.view.zoom = 1.0 / ppp;
            }
        });
        let label = if self.preview.pending.is_some() {
            tr("Preview export (rendering...)")
        } else {
            tr("Preview export")
        };
        if crate::theme::toggle_sized(ui, self.preview.on, label, egui::vec2(0.0, 32.0))
            .on_hover_text(tr("Show exactly what the exported image will look like"))
            .clicked()
        {
            self.preview.on = !self.preview.on;
            self.commit_text();
        }
    }

    fn bar_right(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, env: &Env, exit: &mut Exit) {
        use crate::theme::{Btn, Emphasis};
        let guide_step = self.step.is_some();
        let save = Btn::new(
            if guide_step {
                tr("Save to guide")
            } else {
                tr("Save edit")
            },
            Emphasis::Secondary,
        )
        .compact()
        .chip("Ctrl+S") // i18n-ok
        .name(if guide_step {
            tr("Save to guide (Ctrl+S)")
        } else {
            tr("Save edit (Ctrl+S)")
        })
        .show(ui)
        .on_hover_text(if guide_step {
            tr("Keeps these annotations in the guide step")
        } else {
            tr("Saves the original and your annotations so you can keep editing later")
        });
        if save.clicked() {
            self.save(env);
        }
        let export = ui
            .add_enabled_ui(self.export_rx.is_none(), |ui| {
                Btn::new(tr("Export image"), Emphasis::Accent)
                    .compact()
                    .show(ui)
                    .on_hover_text(tr(
                        "Writes the flattened picture with the annotations burned in (Ctrl+E)",
                    ))
            })
            .inner;
        if export.clicked() {
            self.export(ctx, env);
        }
        if guide_step {
            if Btn::new(tr("Done"), Emphasis::Secondary)
                .compact()
                .chip("Ctrl+W") // i18n-ok
                .name(tr("Done (Ctrl+W)"))
                .show(ui)
                .on_hover_text(tr("Keep the changes and go back to the guide"))
                .clicked()
            {
                self.save(env);
                *exit = Exit::Close;
            }
            if Btn::new(tr("Cancel"), Emphasis::Secondary)
                .compact()
                .show(ui)
                .on_hover_text(tr(
                    "Go back to the guide, discarding changes since the last save",
                ))
                .clicked()
            {
                self.request_close(exit);
            }
        } else if Btn::new(tr("Close"), Emphasis::Secondary)
            .compact()
            .chip("Ctrl+W") // i18n-ok
            .name(tr("Close editor (Ctrl+W)"))
            .show(ui)
            .clicked()
        {
            self.request_close(exit);
        }
    }

    /// The tool tiles on the left.
    fn tool_palette(&mut self, ui: &mut egui::Ui) {
        crate::theme::section_label(ui, tr("Tools"));
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        let width = (ui.available_width() - 8.0) / 2.0;
        for pair in Tool::ALL.chunks(2) {
            ui.horizontal(|ui| {
                for &t in pair {
                    if tools_ui::tool_tile(ui, t, self.tool == t, width).clicked() {
                        self.set_tool(t);
                    }
                }
            });
        }
    }

    /// The panel on the right: what the selected annotation (or, with nothing selected, the
    /// active tool) looks like, and the controls to change it.
    fn props_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        use crate::theme;
        let pal = theme::palette(ctx);
        let selected = self.selected_annotation();
        // What the controls apply to: the selection, otherwise the current tool.
        let kind_tool = match selected.as_ref().map(|a| &a.kind) {
            Some(Kind::Pen { .. }) => Tool::Pen,
            Some(Kind::Highlighter { .. }) => Tool::Highlighter,
            Some(Kind::Line { .. }) => Tool::Line,
            Some(Kind::Arrow { .. }) => Tool::Arrow,
            Some(Kind::Rectangle { .. }) => Tool::Rectangle,
            Some(Kind::Ellipse { .. }) => Tool::Ellipse,
            Some(Kind::Text { .. }) => Tool::Text,
            Some(Kind::Marker { .. }) => Tool::Marker,
            Some(Kind::Obscure { .. }) => Tool::Blur,
            None => self.tool,
        };
        let has_selection = selected.is_some();
        let mut changed = false;

        // The bottom part is laid out first so the controls above can use what is left.
        egui::TopBottomPanel::bottom("editor_props_bottom")
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                ui.add_space(8.0);
                if !self.profile_names.is_empty() {
                    // The picker has its own "Profile" caption.
                    ui.horizontal_wrapped(|ui| {
                        if let Some(r) = self.profile_picker.show(
                            ui,
                            "editor-profile",
                            &self.profile_names,
                            &self.profile_active,
                        ) {
                            self.profile_request = Some(r);
                        }
                    });
                    ui.add_space(10.0);
                }
                if has_selection {
                    let delete = egui::Button::new(
                        egui::RichText::new(tr("Delete annotation")).color(pal.error),
                    )
                    .fill(pal.row)
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .corner_radius(egui::CornerRadius::same(theme::WIDGET_RADIUS))
                    .min_size(egui::vec2(ui.available_width(), 36.0));
                    let clicked = ui
                        .with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.add(delete).clicked()
                        })
                        .inner;
                    if clicked {
                        self.delete_selected();
                    }
                } else {
                    ui.label(
                        egui::RichText::new(tr(
                            "With nothing selected, this panel shows the settings for the active tool.",
                        ))
                        .color(pal.muted),
                    );
                }
            });

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(kind_tool.label())
                    .size(17.0)
                    .family(theme::strong_family()),
            );
            if has_selection {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(tr("selected")).color(pal.muted));
                });
            }
        });
        ui.add_space(12.0);

        let pick_colour = |ui: &mut egui::Ui, props: &mut Props| -> bool {
            let mut changed = false;
            theme::section_label(ui, tr("Colour"));
            ui.add_space(4.0);
            if let Some(c) = tools_ui::swatch_row(ui, props.color) {
                props.color = Rgba([c.0[0], c.0[1], c.0[2], props.color.0[3]]);
                changed = true;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let label = ui.label(egui::RichText::new(tr("Other colour")).color(pal.muted));
                let mut c = to_color32(props.color);
                if ui
                    .color_edit_button_srgba(&mut c)
                    .labelled_by(label.id)
                    .changed()
                {
                    props.color = from_color32(c);
                    changed = true;
                }
            });
            changed
        };
        match kind_tool {
            Tool::Select | Tool::Crop | Tool::Eraser => {
                ui.label(
                    egui::RichText::new(if !has_selection && kind_tool == Tool::Select {
                        tr("Select an annotation to change its colour, width and more.")
                    } else {
                        self.tool.hint()
                    })
                    .color(pal.muted),
                );
            }
            Tool::Blur => {
                theme::section_label(ui, tr("Mode"));
                ui.add_space(4.0);
                theme::segmented(ui, |ui| {
                    for (mode, label) in [
                        (ObscureMode::Pixelate, tr("Pixelate")),
                        (ObscureMode::Blur, tr("Blur")),
                    ] {
                        let on = self.props.obscure_mode == mode;
                        if theme::segment(ui, on, label, egui::vec2(0.0, 28.0)).clicked() && !on {
                            self.props.obscure_mode = mode;
                            changed = true;
                        }
                    }
                });
                ui.add_space(12.0);
                let shown = self.props.obscure_strength.to_string();
                changed |= tools_ui::slider_row(
                    ui,
                    tr("Strength"),
                    &shown,
                    &mut self.props.obscure_strength,
                    2..=60,
                )
                .changed();
            }
            _ => {
                changed |= pick_colour(ui, &mut self.props);
                ui.add_space(12.0);
                match kind_tool {
                    Tool::Highlighter => {
                        let shown = format!("{:.0} px", self.props.highlighter_width); // i18n-ok
                        changed |= tools_ui::slider_row(
                            ui,
                            tr("Width"),
                            &shown,
                            &mut self.props.highlighter_width,
                            6.0..=80.0,
                        )
                        .changed();
                    }
                    Tool::Marker => {
                        let shown = format!("{:.0} px", self.props.marker_radius); // i18n-ok
                        changed |= tools_ui::slider_row(
                            ui,
                            tr("Size"),
                            &shown,
                            &mut self.props.marker_radius,
                            8.0..=60.0,
                        )
                        .changed();
                    }
                    Tool::Text => {
                        let shown = format!("{:.0} px", self.props.text_size); // i18n-ok
                        changed |= tools_ui::slider_row(
                            ui,
                            tr("Text size"),
                            &shown,
                            &mut self.props.text_size,
                            8.0..=160.0,
                        )
                        .changed();
                        ui.add_space(8.0);
                        changed |=
                            theme::checkbox_tile(ui, &mut self.props.bold, tr("Bold")).changed();
                    }
                    _ => {
                        let shown = format!("{:.0} px", self.props.width); // i18n-ok
                        let slider = tools_ui::slider_row(
                            ui,
                            tr("Width"),
                            &shown,
                            &mut self.props.width,
                            1.0..=40.0,
                        );
                        self.width_slider = Some(slider.rect);
                        changed |= slider.changed();
                    }
                }
                if kind_tool != Tool::Highlighter {
                    // The highlighter has its own fixed transparency; everything else uses the
                    // colour's alpha.
                    ui.add_space(12.0);
                    let mut percent = (f32::from(self.props.color.0[3]) / 2.55).round();
                    let shown = format!("{percent:.0}%"); // i18n-ok
                    if tools_ui::slider_row(ui, tr("Opacity"), &shown, &mut percent, 10.0..=100.0)
                        .changed()
                    {
                        self.props.color.0[3] = (percent * 2.55).round().clamp(0.0, 255.0) as u8;
                        changed = true;
                    }
                }
                if matches!(kind_tool, Tool::Rectangle | Tool::Ellipse | Tool::Text) {
                    ui.add_space(12.0);
                    changed |= theme::checkbox_tile(
                        ui,
                        &mut self.props.fill_on,
                        if kind_tool == Tool::Text {
                            tr("Background")
                        } else {
                            tr("Fill shape")
                        },
                    )
                    .changed();
                    if self.props.fill_on {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let label =
                                ui.label(egui::RichText::new(tr("Fill colour")).color(pal.muted));
                            let mut f = to_color32(self.props.fill);
                            if ui
                                .color_edit_button_srgba(&mut f)
                                .labelled_by(label.id)
                                .changed()
                            {
                                self.props.fill = from_color32(f);
                                changed = true;
                            }
                        });
                    }
                }
            }
        }
        if changed {
            self.profile_props = self.props.clone();
            let pointer_down = ctx.input(|i| i.pointer.any_down());
            let continuing = self
                .prop_touch
                .is_some_and(|t| pointer_down || t.elapsed() < Duration::from_millis(600));
            self.prop_touch = Some(Instant::now());
            if let Some(mut a) = self.selected_annotation() {
                self.props.apply(&mut a);
                self.patches.remove(&a.id);
                let changed = if continuing {
                    self.editor.replace_continuing(a)
                } else {
                    self.editor.replace(a)
                };
                self.touched(changed);
            }
        }
    }

    // ---- canvas ------------------------------------------------------------------------

    fn canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let rect = ui.available_rect_before_wrap();
        if self.refit_on_resize {
            if self.auto_view.is_some_and(|v| v != self.view) {
                self.refit_on_resize = false; // the user zoomed or panned: leave the view alone
            } else if self.last_canvas_size != Some(rect.size()) {
                self.needs_fit = true;
            }
            self.last_canvas_size = Some(rect.size());
        }
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Other,
                true,
                tr("Drawing area. Use the toolbar to choose a tool, then drag on the screenshot."),
            )
        });
        let ppp = ctx.pixels_per_point();
        if self.needs_fit {
            self.view = View::fit(rect, self.size, 1.0 / ppp);
            self.auto_view = Some(self.view);
            self.needs_fit = false;
        }
        if resp.hovered() {
            // Ctrl+scroll (and a pinch) zooms around the pointer; plain scroll moves the picture,
            // sideways too (Shift+scroll on a mouse wheel), like in other image viewers.
            let (scroll, zoom, pointer) =
                ctx.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
            if let (true, Some(p)) = (zoom != 1.0, pointer) {
                self.view.zoom_at(p, zoom);
            } else if scroll != egui::Vec2::ZERO {
                self.view.pan(scroll);
            }
        }
        self.interact(ctx, &resp);
        self.ensure_patches(ctx);

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, crate::theme::palette(ui.ctx()).canvas);
        self.paint_canvas(&painter);
    }

    fn pointer_image(&self, resp: &egui::Response) -> Option<Point> {
        resp.hover_pos()
            .or_else(|| resp.interact_pointer_pos())
            .map(|p| self.view.to_image(p))
    }

    fn handle_hit(&self, a: &Annotation, screen: Pos2) -> Option<Handle> {
        self.handle_of(exact_bounds(a), screen)
    }

    fn handle_of(&self, bounds: RectF, screen: Pos2) -> Option<Handle> {
        Handle::ALL
            .into_iter()
            .find(|h| self.view.to_screen(h.position(bounds)).distance(screen) <= 8.0)
    }

    /// The crop as a rectangle, when there is one.
    fn crop_rect(&self) -> Option<RectF> {
        self.editor.document().crop.map(|c| RectF {
            x: c.x as f32,
            y: c.y as f32,
            w: c.w as f32,
            h: c.h as f32,
        })
    }

    /// The handle of the crop under `screen`, only while the Crop tool is chosen.
    fn crop_handle_hit(&self, screen: Pos2) -> Option<Handle> {
        if self.tool != Tool::Crop {
            return None;
        }
        self.handle_of(self.crop_rect()?, screen)
    }

    /// Sets the crop from a drag, or says why the dragged area was not used.
    fn finish_crop(&mut self, area: RectF) {
        match clipped_pixels(area, self.size).filter(|r| r.w >= 2 && r.h >= 2) {
            Some(r) => {
                self.set_crop(Some(r));
            }
            None => self.status = tr("That area is too small to crop; drag a larger one").into(),
        }
    }

    fn interact(&mut self, ctx: &egui::Context, resp: &egui::Response) {
        if resp.dragged_by(PointerButton::Middle) {
            let mut delta = resp.drag_delta();
            if resp.drag_started_by(PointerButton::Middle) {
                // egui starts the drag only after a few pixels of movement; count those too.
                if let (Some(origin), Some(now)) = (
                    ctx.input(|i| i.pointer.press_origin()),
                    resp.interact_pointer_pos(),
                ) {
                    delta = now - origin;
                }
            }
            self.view.pan(delta);
            ctx.set_cursor_icon(CursorIcon::Grabbing);
            return;
        }
        if self.preview.on {
            return; // the preview is read-only; zoom and pan still work
        }
        let tol = 6.0 / self.view.zoom;
        if resp.drag_started_by(PointerButton::Primary) {
            // egui reports the drag only after the pointer has moved a little; the
            // gesture must start where the button went down.
            let origin = ctx
                .input(|i| i.pointer.press_origin())
                .or(resp.interact_pointer_pos());
            if let Some(o) = origin {
                self.begin_gesture(ctx, o, tol);
                // Space+drag panning: include the movement before egui noticed the drag.
                if matches!(self.gesture, Some(Gesture::Pan)) {
                    if let Some(now) = resp.interact_pointer_pos() {
                        self.view.pan(now - o - resp.drag_delta());
                    }
                }
            }
        }
        if resp.dragged_by(PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                self.update_gesture(p, resp.drag_delta(), tol);
            }
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            self.end_gesture();
        }
        if resp.double_clicked_by(PointerButton::Primary) && self.tool == Tool::Select {
            if let Some(a) = self.selected_annotation() {
                if let Kind::Text { at, text, .. } = &a.kind {
                    self.start_text(*at, text.clone(), Some(a.id));
                }
            }
        } else if resp.clicked_by(PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                self.on_click(self.view.to_image(p), tol);
            }
        }
        self.hover_cursor(ctx, resp, tol);
    }

    fn hover_cursor(&self, ctx: &egui::Context, resp: &egui::Response, tol: f32) {
        if !resp.hovered() || self.gesture.is_some() {
            return;
        }
        if self.tool != Tool::Select {
            let over_crop_handle = resp
                .hover_pos()
                .and_then(|screen| self.crop_handle_hit(screen));
            ctx.set_cursor_icon(if let Some(h) = over_crop_handle {
                resize_cursor(h)
            } else if self.tool == Tool::Text {
                CursorIcon::Text
            } else {
                CursorIcon::Crosshair
            });
            return;
        }
        let (Some(p), Some(screen)) = (self.pointer_image(resp), resp.hover_pos()) else {
            return;
        };
        if let Some(h) = self
            .selected_annotation()
            .and_then(|a| self.handle_hit(&a, screen))
        {
            ctx.set_cursor_icon(resize_cursor(h));
        } else if topmost_at(self.editor.document(), p, tol).is_some() {
            ctx.set_cursor_icon(CursorIcon::Move);
        }
    }

    fn begin_gesture(&mut self, ctx: &egui::Context, screen: Pos2, tol: f32) {
        let p = self.view.to_image(screen);
        if ctx.input(|i| i.key_down(Key::Space)) {
            self.gesture = Some(Gesture::Pan);
            return;
        }
        self.commit_text();
        match self.tool {
            Tool::Select => {
                if let Some(a) = self.selected_annotation() {
                    if let Some(handle) = self.handle_hit(&a, screen) {
                        let bounds = exact_bounds(&a);
                        self.gesture = Some(Gesture::Resize {
                            current: a.clone(),
                            original: a,
                            handle,
                            bounds,
                        });
                        return;
                    }
                }
                if let Some(id) = topmost_at(self.editor.document(), p, tol) {
                    self.select(Some(id));
                    if let Some(a) = self.selected_annotation() {
                        self.gesture = Some(Gesture::Move {
                            current: a.clone(),
                            original: a,
                            start: p,
                        });
                    }
                } else {
                    self.selected = None;
                }
            }
            Tool::Pen | Tool::Highlighter => {
                self.gesture = Some(Gesture::Draw {
                    tool: self.tool,
                    start: p,
                    cur: p,
                    points: vec![p],
                });
            }
            Tool::Crop if self.crop_handle_hit(screen).is_some() => {
                let handle = self.crop_handle_hit(screen).expect("checked above");
                let original = self.crop_rect().expect("a handle was hit");
                self.gesture = Some(Gesture::CropResize {
                    original,
                    handle,
                    current: original,
                });
            }
            Tool::Line
            | Tool::Arrow
            | Tool::Rectangle
            | Tool::Ellipse
            | Tool::Blur
            | Tool::Crop => {
                self.gesture = Some(Gesture::Draw {
                    tool: self.tool,
                    start: p,
                    cur: p,
                    points: Vec::new(),
                });
            }
            Tool::Eraser => {
                self.gesture = Some(Gesture::Erase);
                self.erase_at(p, tol);
            }
            Tool::Text | Tool::Marker => {}
        }
    }

    fn update_gesture(&mut self, screen: Pos2, delta: egui::Vec2, tol: f32) {
        let q = self.view.to_image(screen);
        let zoom = self.view.zoom;
        match &mut self.gesture {
            Some(Gesture::Draw { cur, points, .. }) => {
                *cur = q;
                if let Some(last) = points.last() {
                    if (last.x - q.x).hypot(last.y - q.y) * zoom >= 1.5 {
                        points.push(q);
                    }
                }
            }
            Some(Gesture::Move {
                original,
                start,
                current,
            }) => {
                *current = original.clone();
                current.translate(q.x - start.x, q.y - start.y);
            }
            Some(Gesture::Resize {
                original,
                handle,
                bounds,
                current,
            }) => {
                *current = original.clone();
                current.transform_to(*bounds, resize_rect(*bounds, *handle, q));
            }
            Some(Gesture::CropResize {
                original,
                handle,
                current,
            }) => *current = resize_rect(*original, *handle, q),
            Some(Gesture::Pan) => self.view.pan(delta),
            Some(Gesture::Erase) => self.erase_at(q, tol),
            None => {}
        }
    }

    fn end_gesture(&mut self) {
        let Some(g) = self.gesture.take() else { return };
        match g {
            Gesture::Draw {
                tool: Tool::Crop,
                start,
                cur,
                ..
            } => {
                self.finish_crop(RectF::from_points(start, cur));
            }
            Gesture::CropResize { current, .. } => self.finish_crop(current),
            Gesture::Draw {
                tool,
                start,
                cur,
                mut points,
            } => {
                if matches!(tool, Tool::Pen | Tool::Highlighter) {
                    if points.last().map(|l| (l.x - cur.x).hypot(l.y - cur.y)) > Some(0.0) {
                        points.push(cur);
                    }
                    points.dedup();
                }
                if let Some(a) = drafted(tool, start, cur, &points, &self.props) {
                    let id = self.add(a.kind, a.style);
                    if !matches!(tool, Tool::Pen | Tool::Highlighter) {
                        self.select(Some(id));
                        self.tool = Tool::Select;
                    }
                }
            }
            Gesture::Move {
                original, current, ..
            }
            | Gesture::Resize {
                original, current, ..
            } => {
                if current != original {
                    self.replace(current);
                }
            }
            Gesture::Pan | Gesture::Erase => {}
        }
    }

    fn on_click(&mut self, p: Point, tol: f32) {
        match self.tool {
            Tool::Select => {
                let id = topmost_at(self.editor.document(), p, tol);
                self.select(id);
            }
            Tool::Pen | Tool::Highlighter => {
                let tool = self.tool;
                if let Some(a) = drafted(tool, p, p, &[p], &self.props) {
                    self.add(a.kind, a.style);
                }
            }
            Tool::Marker => {
                let id = self.editor.add_marker(
                    p,
                    self.props.marker_radius,
                    self.props.style_for(false),
                );
                self.touched(true);
                self.select(Some(id));
            }
            Tool::Text => self.start_text(p, String::new(), None),
            Tool::Eraser => self.erase_at(p, tol),
            Tool::Line
            | Tool::Arrow
            | Tool::Rectangle
            | Tool::Ellipse
            | Tool::Blur
            | Tool::Crop => {}
        }
    }

    fn erase_at(&mut self, p: Point, tol: f32) {
        if let Some(id) = topmost_at(self.editor.document(), p, tol) {
            self.remove(id);
        }
    }

    fn text_popup(&mut self, ctx: &egui::Context) {
        let Some(t) = &mut self.text else { return };
        let pos = self.view.to_screen(t.at);
        let mut done = false;
        let mut cancel = false;
        egui::Area::new(egui::Id::new("editor_text"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let edit = ui.add(
                        crate::theme::text_multiline(&mut t.text)
                            .desired_width(240.0)
                            .desired_rows(2)
                            .hint_text(tr(
                                "Type text. Ctrl+Enter or click away to finish, Esc to cancel",
                            )),
                    );
                    if t.fresh {
                        edit.request_focus();
                        t.fresh = false;
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        cancel = true;
                    } else if ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter))
                        || edit.lost_focus()
                    {
                        done = true;
                    }
                });
            });
        if cancel {
            self.text = None;
        } else if done {
            self.commit_text();
        }
    }

    fn ensure_patches(&mut self, ctx: &egui::Context) {
        if self.preview.on {
            return;
        }
        let moving = match &self.gesture {
            Some(Gesture::Move { original, .. }) | Some(Gesture::Resize { original, .. }) => {
                Some(original.id)
            }
            _ => None,
        };
        for a in &self.editor.document().annotations {
            if Some(a.id) == moving || !matches!(a.kind, Kind::Obscure { .. }) {
                continue;
            }
            if self
                .patches
                .get(&a.id)
                .is_some_and(|(k, _, _)| *k == a.kind)
            {
                continue;
            }
            if let Some((img, region)) = obscure_patch(&self.original, a) {
                self.patches.insert(
                    a.id,
                    (
                        a.kind.clone(),
                        texture_from(ctx, "editor-patch", img),
                        region,
                    ),
                );
            }
        }
    }

    fn paint_canvas(&self, painter: &egui::Painter) {
        let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let image_rect = self.view.screen_rect(RectF {
            x: 0.0,
            y: 0.0,
            w: self.size.0 as f32,
            h: self.size.1 as f32,
        });
        if self.preview.on {
            if let Some(tex) = &self.preview.tex {
                let cropped = self.editor.document().crop;
                let (w, h) = cropped.map_or(self.size, |c| (c.w, c.h));
                let r = self.view.screen_rect(RectF {
                    x: self.preview.offset.x,
                    y: self.preview.offset.y,
                    w: w as f32,
                    h: h as f32,
                });
                painter.image(tex.id(), r, full_uv, Color32::WHITE);
            } else {
                painter.image(
                    self.tex.id(),
                    image_rect,
                    full_uv,
                    Color32::from_white_alpha(80),
                );
                painter.text(
                    image_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    tr("Rendering preview..."),
                    egui::FontId::proportional(18.0),
                    Color32::WHITE,
                );
            }
            return;
        }
        painter.image(self.tex.id(), image_rect, full_uv, Color32::WHITE);
        let overridden = match &self.gesture {
            Some(Gesture::Move { current, .. }) | Some(Gesture::Resize { current, .. }) => {
                Some(current)
            }
            _ => None,
        };
        for a in &self.editor.document().annotations {
            let shown = overridden.filter(|o| o.id == a.id).unwrap_or(a);
            let patch = self
                .patches
                .get(&a.id)
                .filter(|_| overridden.is_none_or(|o| o.id != a.id))
                .map(|(_, t, r)| (t, *r));
            paint::paint_annotation(painter, &self.view, shown, patch);
        }
        if let Some(Gesture::Draw {
            tool,
            start,
            cur,
            points,
        }) = &self.gesture
        {
            if *tool == Tool::Crop {
                let r = RectF::from_points(*start, *cur);
                painter.rect_stroke(
                    self.view.screen_rect(r),
                    0.0,
                    egui::Stroke::new(1.5, Color32::YELLOW),
                    egui::StrokeKind::Outside,
                );
            } else if let Some(a) = drafted(*tool, *start, *cur, points, &self.props) {
                paint::paint_annotation(painter, &self.view, &a, None);
            }
        }
        if let Some(crop) = self.crop_rect() {
            paint::paint_crop(painter, &self.view, self.size, crop);
            if self.tool == Tool::Crop {
                match &self.gesture {
                    Some(Gesture::CropResize { current, .. }) => {
                        paint::paint_handles(painter, &self.view, *current)
                    }
                    Some(_) => {}
                    None => paint::paint_handles(painter, &self.view, crop),
                }
            }
        }
        if self.tool == Tool::Select {
            let target = overridden.cloned().or_else(|| self.selected_annotation());
            if let Some(a) = target {
                paint::paint_handles(painter, &self.view, exact_bounds(&a));
            }
        }
    }

    /// Test hook: describes the view (`origin` is where image pixel (0, 0) is on the screen,
    /// `scale` is physical screen pixels per image pixel) so end-to-end tests can map screen to image pixels.
    fn write_report(&mut self, ctx: &egui::Context) {
        let Some(path) = &self.report_path else {
            return;
        };
        let ppp = ctx.pixels_per_point();
        // The view origin is relative to the window; add the window's own screen position.
        let window = ctx
            .input(|i| i.viewport().inner_rect)
            .map_or((0.0, 0.0), |r| (r.min.x * ppp, r.min.y * ppp));
        let doc = self.editor.document();
        let report = format!(
            "{{\"origin\":[{:.2},{:.2}],\"scale\":{:.5},\"annotations\":{},\"selected\":{},\"crop\":{},\"dirty\":{},\"confirm_close\":{},\"slider\":{},\"status\":{:?}}}",
            window.0 + self.view.origin.x * ppp,
            window.1 + self.view.origin.y * ppp,
            self.view.zoom * ppp,
            doc.annotations.len(),
            self.selected.map_or(0, |i| i.0),
            doc.crop.is_some(),
            self.is_dirty(),
            self.confirm_close,
            self.width_slider.map_or_else(
                || "null".to_string(),
                |r| format!(
                    "[{:.1},{:.1},{:.1},{:.1}]",
                    window.0 + r.min.x * ppp,
                    window.1 + r.min.y * ppp,
                    r.width() * ppp,
                    r.height() * ppp
                )
            ),
            self.status,
        );
        if report != self.last_report {
            let _ = std::fs::write(path, &report);
            self.last_report = report;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guide_core::Rgba;
    use std::fs;

    fn ann(kind: Kind) -> Annotation {
        Annotation {
            id: AnnotationId(1),
            kind,
            style: Style::default(),
        }
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> RectF {
        RectF { x, y, w, h }
    }

    #[test]
    fn undo_reloads_the_controls_from_the_selected_annotation() {
        let ctx = egui::Context::default();
        let shot = Screenshot {
            width: 20,
            height: 20,
            rgba: vec![255; 20 * 20 * 4],
        };
        let mut v = EditorView::new(&ctx, &shot, Document::default(), None, "t".into());
        let id = v.add(
            Kind::Rectangle {
                rect: rect(1.0, 1.0, 5.0, 5.0),
            },
            Style::default(),
        );
        v.select(Some(id));
        assert_eq!(v.props.width, 4.0);
        // The user drags the width slider to 30: controls and annotation change together.
        v.props.width = 30.0;
        let mut a = v.selected_annotation().unwrap();
        v.props.apply(&mut a);
        v.replace(a);
        assert_eq!(v.editor.document().find(id).unwrap().style.width, 30.0);
        v.undo();
        assert_eq!(v.props.width, 4.0, "the width control must follow the undo");
        v.redo();
        assert_eq!(v.props.width, 30.0, "and the redo");
    }

    /// Runs `handle_keys` for one frame in which `key` goes down (with Ctrl when `ctrl`).
    fn press(v: &mut EditorView, ctx: &egui::Context, key: Key, ctrl: bool) {
        press_in(v, ctx, key, ctrl, false);
    }

    fn press_in(v: &mut EditorView, ctx: &egui::Context, key: Key, ctrl: bool, gui: bool) {
        let env = Env {
            out_dir: PathBuf::new(),
            format: ImageFormat::Png,
            gui,
        };
        let modifiers = if ctrl {
            egui::Modifiers::COMMAND
        } else {
            egui::Modifiers::NONE
        };
        let input = egui::RawInput {
            modifiers,
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| v.handle_keys(ctx, &env));
    }

    fn view_with_a_selected_rectangle(ctx: &egui::Context) -> (EditorView, AnnotationId) {
        let shot = Screenshot {
            width: 20,
            height: 20,
            rgba: vec![255; 20 * 20 * 4],
        };
        let mut v = EditorView::new(ctx, &shot, Document::default(), None, "t".into());
        let id = v.add(
            Kind::Rectangle {
                rect: rect(1.0, 1.0, 5.0, 5.0),
            },
            Style::default(),
        );
        v.select(Some(id));
        (v, id)
    }

    #[test]
    fn delete_and_undo_keys_edit_the_document_when_there_is_no_preview() {
        let ctx = egui::Context::default();
        let (mut v, id) = view_with_a_selected_rectangle(&ctx);
        press(&mut v, &ctx, Key::Delete, false);
        assert!(v.editor.document().find(id).is_none());
        press(&mut v, &ctx, Key::Z, true);
        assert!(
            v.editor.document().find(id).is_some(),
            "undo brings it back"
        );
    }

    #[test]
    fn redo_and_delete_first_finish_a_text_that_is_being_typed() {
        let ctx = egui::Context::default();
        let (mut v, id) = view_with_a_selected_rectangle(&ctx);
        v.start_text(Point { x: 2.0, y: 2.0 }, "typed".into(), None);
        v.redo();
        assert!(v.text.is_none(), "redo finishes the text first");
        let texts = |v: &EditorView| {
            v.editor
                .document()
                .annotations
                .iter()
                .filter(|a| matches!(a.kind, Kind::Text { .. }))
                .count()
        };
        assert_eq!(texts(&v), 1, "the typed text became an annotation");
        // Editing an existing text, then deleting: the popup must not outlive the annotation.
        let text_id = v.selected.unwrap();
        v.start_text(Point { x: 2.0, y: 2.0 }, "changed".into(), Some(text_id));
        v.delete_selected();
        assert!(v.text.is_none());
        assert_eq!(texts(&v), 0, "deleted, not resurrected by a stale popup");
        assert!(v.editor.document().find(id).is_some());
    }

    #[test]
    fn editing_keys_do_nothing_while_the_export_preview_is_showing() {
        let ctx = egui::Context::default();
        let (mut v, id) = view_with_a_selected_rectangle(&ctx);
        v.preview.on = true;
        press(&mut v, &ctx, Key::Delete, false);
        assert!(v.editor.document().find(id).is_some(), "Delete is ignored");
        let tool = v.tool;
        let other = Tool::ALL.into_iter().find(|t| *t != tool).unwrap();
        press(&mut v, &ctx, other.key(), false);
        assert_eq!(v.tool, tool, "tool hotkeys are ignored");
        v.preview.on = false;
        press(&mut v, &ctx, Key::Delete, false);
        assert!(v.editor.document().find(id).is_none());
        v.preview.on = true;
        press(&mut v, &ctx, Key::Z, true);
        assert!(v.editor.document().find(id).is_none(), "Ctrl+Z is ignored");
        press(&mut v, &ctx, Key::Y, true);
        assert!(v.editor.document().find(id).is_none(), "Ctrl+Y is ignored");
        assert!(v.editor.can_undo(), "the history is intact");
    }

    fn blank_view(ctx: &egui::Context) -> EditorView {
        let shot = Screenshot {
            width: 20,
            height: 20,
            rgba: vec![255; 20 * 20 * 4],
        };
        EditorView::new(ctx, &shot, Document::default(), None, "shot".into())
    }

    fn env_in(dir: &std::path::Path, gui: bool) -> Env {
        Env {
            out_dir: dir.to_path_buf(),
            format: ImageFormat::Png,
            gui,
        }
    }

    fn texts(v: &EditorView) -> Vec<String> {
        v.editor
            .document()
            .annotations
            .iter()
            .filter_map(|a| match &a.kind {
                Kind::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn committing_a_typed_text_adds_it_and_returns_to_the_select_tool() {
        let ctx = egui::Context::default();
        let mut v = blank_view(&ctx);
        v.set_tool(Tool::Text);
        v.start_text(Point { x: 3.0, y: 3.0 }, "hello".into(), None);
        v.commit_text();
        assert_eq!(texts(&v), ["hello"]);
        assert_eq!(v.tool, Tool::Select);
        assert!(v.selected.is_some() && v.text.is_none());
    }

    #[test]
    fn committing_an_emptied_text_removes_it_and_a_blank_new_text_adds_nothing() {
        let ctx = egui::Context::default();
        let mut v = blank_view(&ctx);
        v.start_text(Point { x: 3.0, y: 3.0 }, "hello".into(), None);
        v.commit_text();
        let id = v.selected.unwrap();
        v.start_text(Point { x: 3.0, y: 3.0 }, "  ".into(), Some(id));
        v.commit_text();
        assert!(texts(&v).is_empty(), "emptying a text removes it");
        v.start_text(Point { x: 3.0, y: 3.0 }, String::new(), None);
        v.commit_text();
        assert!(texts(&v).is_empty());
    }

    #[test]
    fn saving_a_step_edit_hands_the_document_back_and_writes_no_files() {
        let ctx = egui::Context::default();
        let shot = Screenshot {
            width: 20,
            height: 20,
            rgba: vec![255; 20 * 20 * 4],
        };
        let mut v = EditorView::for_step(&ctx, &shot, Document::default(), ItemId(4));
        v.add(
            Kind::Rectangle {
                rect: rect(1.0, 1.0, 5.0, 5.0),
            },
            Style::default(),
        );
        assert!(v.is_dirty());
        let dir = std::env::temp_dir().join(format!("sg-editor-step-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        v.save(&env_in(&dir, false));
        assert!(!v.is_dirty());
        let (step, doc) = v.take_commit().unwrap();
        assert_eq!((step, doc.annotations.len()), (ItemId(4), 1));
        assert!(v.take_commit().is_none(), "handed back once");
        assert!(
            !dir.exists(),
            "the editor itself writes nothing in step mode"
        );
    }

    #[test]
    fn saving_an_edit_bundle_picks_a_new_folder_name_and_clears_the_dirty_flag() {
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir().join(format!("sg-editor-bundle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(format!("shot.{}", guide_core::EDIT_EXTENSION))).unwrap(); // the first name is taken
        let mut v = blank_view(&ctx);
        v.add(
            Kind::Rectangle {
                rect: rect(1.0, 1.0, 5.0, 5.0),
            },
            Style::default(),
        );
        v.save(&env_in(&dir, false));
        assert!(!v.is_dirty());
        assert_eq!(
            v.saved_dir(),
            Some(
                dir.join(format!("shot-2.{}", guide_core::EDIT_EXTENSION))
                    .as_path()
            )
        );
        // A second save writes into the same folder instead of making another.
        v.add(
            Kind::Rectangle {
                rect: rect(2.0, 2.0, 5.0, 5.0),
            },
            Style::default(),
        );
        assert!(v.is_dirty());
        v.save(&env_in(&dir, false));
        assert!(!v.is_dirty());
        assert_eq!(
            v.saved_dir(),
            Some(
                dir.join(format!("shot-2.{}", guide_core::EDIT_EXTENSION))
                    .as_path()
            )
        );
        assert!(!dir
            .join(format!("shot-3.{}", guide_core::EDIT_EXTENSION))
            .exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undoing_back_to_the_saved_state_is_not_an_unsaved_change() {
        let ctx = egui::Context::default();
        let mut v = blank_view(&ctx);
        assert!(!v.is_dirty());
        v.add(
            Kind::Rectangle {
                rect: rect(1.0, 1.0, 5.0, 5.0),
            },
            Style::default(),
        );
        assert!(v.is_dirty());
        v.undo();
        assert!(!v.is_dirty(), "back to what was opened");
        v.redo();
        assert!(v.is_dirty());
        let dir = std::env::temp_dir().join(format!("sg-editor-dirty-{}", std::process::id()));
        v.save(&env_in(&dir, false));
        assert!(!v.is_dirty());
        v.undo();
        assert!(
            v.is_dirty(),
            "the saved file has the rectangle, the view does not"
        );
        v.redo();
        assert!(!v.is_dirty(), "and back to the saved state again");
        let mut exit = Exit::Stay;
        v.request_close(&mut exit);
        assert!(matches!(exit, Exit::Close), "nothing to ask about");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Drags from `from` to `to` (screen = image coordinates at zoom 1) with the current tool.
    fn drag(v: &mut EditorView, ctx: &egui::Context, from: Pos2, to: Pos2) {
        v.begin_gesture(ctx, from, 4.0);
        v.update_gesture(to, egui::Vec2::ZERO, 4.0);
        v.end_gesture();
    }

    fn crop_of(v: &EditorView) -> Option<(u32, u32, u32, u32)> {
        v.editor.document().crop.map(|c| (c.x, c.y, c.w, c.h))
    }

    /// A 300x300 white image shown at zoom 1, big enough to keep the crop handles apart.
    fn big_view(ctx: &egui::Context) -> EditorView {
        let shot = Screenshot {
            width: 300,
            height: 300,
            rgba: vec![255; 300 * 300 * 4],
        };
        EditorView::new(ctx, &shot, Document::default(), None, "big".into())
    }

    fn at(x: f32, y: f32) -> Pos2 {
        Pos2::new(x, y)
    }

    #[test]
    fn a_drawn_crop_can_be_adjusted_by_its_handles() {
        let ctx = egui::Context::default();
        let mut v = big_view(&ctx);
        v.set_tool(Tool::Crop);
        drag(&mut v, &ctx, at(40.0, 40.0), at(120.0, 120.0));
        assert_eq!(crop_of(&v), Some((40, 40, 80, 80)));
        // Grab the south-east corner and pull it out.
        drag(&mut v, &ctx, at(120.0, 120.0), at(160.0, 180.0));
        assert_eq!(crop_of(&v), Some((40, 40, 120, 140)));
        // The middle of the west edge moves only that edge.
        drag(&mut v, &ctx, at(40.0, 110.0), at(10.0, 110.0));
        assert_eq!(crop_of(&v), Some((10, 40, 150, 140)));
        // One undo step per adjustment.
        v.undo();
        assert_eq!(crop_of(&v), Some((40, 40, 120, 140)));
    }

    #[test]
    fn dragging_a_crop_handle_beyond_the_image_stays_inside_it() {
        let ctx = egui::Context::default();
        let mut v = big_view(&ctx);
        v.set_tool(Tool::Crop);
        drag(&mut v, &ctx, at(40.0, 40.0), at(120.0, 120.0));
        drag(&mut v, &ctx, at(120.0, 120.0), at(900.0, 900.0));
        assert_eq!(crop_of(&v), Some((40, 40, 260, 260)));
    }

    #[test]
    fn a_crop_too_small_to_use_is_refused_with_a_hint_and_the_old_crop_stays() {
        let ctx = egui::Context::default();
        let hint = tr("That area is too small to crop; drag a larger one");
        let mut v = big_view(&ctx);
        v.set_tool(Tool::Crop);
        drag(&mut v, &ctx, at(40.0, 40.0), at(120.0, 120.0));
        v.status.clear();
        drag(&mut v, &ctx, at(120.0, 120.0), at(40.5, 40.5));
        assert_eq!(crop_of(&v), Some((40, 40, 80, 80)), "the old crop is kept");
        assert_eq!(v.status, hint);
        // The same for a brand new tiny drag when there is no crop yet.
        let mut v = big_view(&ctx);
        v.set_tool(Tool::Crop);
        drag(&mut v, &ctx, at(50.0, 50.0), at(50.5, 50.5));
        assert_eq!(crop_of(&v), None);
        assert_eq!(v.status, hint);
    }

    #[test]
    fn a_drag_away_from_the_handles_still_draws_a_new_crop() {
        let ctx = egui::Context::default();
        let mut v = big_view(&ctx);
        v.set_tool(Tool::Crop);
        drag(&mut v, &ctx, at(40.0, 40.0), at(80.0, 80.0));
        drag(&mut v, &ctx, at(200.0, 20.0), at(290.0, 150.0));
        assert_eq!(crop_of(&v), Some((200, 20, 90, 130)));
    }

    #[test]
    fn handles_only_grab_while_the_crop_tool_is_chosen() {
        let ctx = egui::Context::default();
        let mut v = big_view(&ctx);
        v.set_tool(Tool::Crop);
        drag(&mut v, &ctx, at(40.0, 40.0), at(120.0, 120.0));
        assert!(v.crop_handle_hit(at(120.0, 120.0)).is_some());
        assert!(v.crop_handle_hit(at(200.0, 200.0)).is_none());
        v.set_tool(Tool::Select);
        assert!(v.crop_handle_hit(at(120.0, 120.0)).is_none());
    }

    /// Shows the canvas for a few frames; the pointer sits over it and, in the first frame, the
    /// mouse wheel turns by `wheel` (with Ctrl when `ctrl`).
    fn scroll_over_canvas(v: &mut EditorView, wheel: egui::Vec2, ctrl: bool) {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let modifiers = if ctrl {
            egui::Modifiers::COMMAND
        } else {
            egui::Modifiers::NONE
        };
        for frame in 0..8 {
            let events = if frame == 0 {
                vec![
                    egui::Event::PointerMoved(Pos2::new(300.0, 300.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: wheel,
                        modifiers,
                    },
                ]
            } else {
                vec![]
            };
            let input = egui::RawInput {
                screen_rect: Some(screen),
                modifiers,
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| v.canvas(ui, ctx));
            });
        }
    }

    #[test]
    fn plain_scroll_moves_the_picture_and_ctrl_scroll_zooms() {
        let ctx = egui::Context::default();
        let mut v = big_view(&ctx);
        v.needs_fit = false;
        v.view = View {
            origin: Pos2::new(50.0, 50.0),
            zoom: 1.0,
        };
        scroll_over_canvas(&mut v, egui::vec2(0.0, 60.0), false);
        assert_eq!(v.view.zoom, 1.0, "a plain scroll does not zoom");
        assert!(
            v.view.origin.y > 60.0,
            "it scrolls the picture: {:?}",
            v.view.origin
        );

        let mut v = big_view(&ctx);
        v.needs_fit = false;
        v.view = View {
            origin: Pos2::new(50.0, 50.0),
            zoom: 1.0,
        };
        scroll_over_canvas(&mut v, egui::vec2(0.0, 60.0), true);
        assert!(v.view.zoom > 1.0, "Ctrl+scroll zooms in: {}", v.view.zoom);
    }

    #[test]
    fn closing_asks_only_when_there_is_something_to_lose() {
        let ctx = egui::Context::default();
        let mut v = blank_view(&ctx);
        let mut exit = Exit::Stay;
        v.request_close(&mut exit);
        assert!(matches!(exit, Exit::Close) && !v.confirm_close);

        let mut v = blank_view(&ctx);
        v.add(
            Kind::Rectangle {
                rect: rect(1.0, 1.0, 5.0, 5.0),
            },
            Style::default(),
        );
        let mut exit = Exit::Stay;
        v.request_close(&mut exit);
        assert!(matches!(exit, Exit::Stay) && v.confirm_close, "asks first");
    }

    #[test]
    fn closing_while_typing_counts_the_text_as_a_change() {
        let ctx = egui::Context::default();
        let mut v = blank_view(&ctx);
        v.start_text(Point { x: 1.0, y: 1.0 }, "unsaved".into(), None);
        let mut exit = Exit::Stay;
        v.request_close(&mut exit);
        assert!(matches!(exit, Exit::Stay) && v.confirm_close);
        assert_eq!(texts(&v), ["unsaved"]);
    }

    #[test]
    fn escape_deselects_first_and_only_the_gui_editor_is_left_by_a_further_escape() {
        let ctx = egui::Context::default();
        let (mut v, _) = view_with_a_selected_rectangle(&ctx);
        press(&mut v, &ctx, Key::Escape, false);
        assert!(v.selected.is_none(), "first Escape deselects");
        assert!(!v.close_key);
        // In the normal editor Escape never closes it.
        press(&mut v, &ctx, Key::Escape, false);
        assert!(!v.close_key);

        let (mut v, _) = view_with_a_selected_rectangle(&ctx);
        press_in(&mut v, &ctx, Key::Escape, false, true);
        assert!(v.selected.is_none() && !v.close_key);
        press_in(&mut v, &ctx, Key::Escape, false, true);
        assert!(v.close_key, "nothing left to cancel: leave the capture");
    }

    #[test]
    fn unsaved_changes_ask_before_the_window_closes() {
        let ctx = egui::Context::default();
        let shot = Screenshot {
            width: 20,
            height: 20,
            rgba: vec![255; 20 * 20 * 4],
        };
        let mut v = EditorView::new(&ctx, &shot, Document::default(), None, "t".into());
        assert!(
            !v.on_window_close_requested(),
            "nothing to lose: close straight away"
        );
        v.add(
            Kind::Rectangle {
                rect: rect(1.0, 1.0, 5.0, 5.0),
            },
            Style::default(),
        );
        assert!(v.is_dirty());
        assert!(
            v.on_window_close_requested(),
            "unsaved: cancel the close and ask"
        );
        assert!(v.confirm_close && v.quit_requested());
    }

    #[test]
    fn text_uses_its_measured_bounds_for_hit_testing() {
        let a = ann(Kind::Text {
            at: Point::new(100.0, 100.0),
            text: "Hello world".into(),
            size: 30.0,
            bold: false,
        });
        let b = exact_bounds(&a);
        assert!(b.w > 100.0 && b.h > 25.0, "{b:?}");
        assert!(hit(
            &a,
            Point::new(100.0 + b.w - 2.0, 100.0 + b.h / 2.0),
            0.0
        ));
        assert!(!hit(&a, Point::new(100.0 + b.w + 30.0, 110.0), 4.0));
    }

    #[test]
    fn topmost_picks_the_last_drawn_annotation_under_the_pointer() {
        let mut e = Editor::new(Document::default());
        let filled = Style {
            fill: Some(Rgba::WHITE),
            ..Style::default()
        };
        let below = e.add(
            Kind::Rectangle {
                rect: rect(0.0, 0.0, 100.0, 100.0),
            },
            filled.clone(),
        );
        let above = e.add(
            Kind::Rectangle {
                rect: rect(50.0, 50.0, 100.0, 100.0),
            },
            filled,
        );
        assert_eq!(
            topmost_at(e.document(), Point::new(60.0, 60.0), 0.0),
            Some(above)
        );
        assert_eq!(
            topmost_at(e.document(), Point::new(10.0, 10.0), 0.0),
            Some(below)
        );
        assert_eq!(
            topmost_at(e.document(), Point::new(500.0, 500.0), 0.0),
            None
        );
    }

    #[test]
    fn clipped_pixels_stay_inside_the_image() {
        assert_eq!(
            clipped_pixels(rect(-5.0, 10.2, 50.0, 20.0), (40, 100)),
            Some(PixelRect {
                x: 0,
                y: 10,
                w: 40,
                h: 21
            })
        );
        assert_eq!(
            clipped_pixels(rect(200.0, 0.0, 10.0, 10.0), (100, 100)),
            None
        );
        assert_eq!(
            clipped_pixels(rect(-50.0, -50.0, 10.0, 10.0), (100, 100)),
            None
        );
    }

    fn striped(w: u32, h: u32) -> Image {
        let rgba = (0..w * h)
            .flat_map(|i| {
                if (i % w) % 2 == 0 {
                    [0, 0, 0, 255]
                } else {
                    [255, 255, 255, 255]
                }
            })
            .collect();
        Image {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn obscure_patch_covers_only_the_clipped_region_and_hides_the_pattern() {
        let img = striped(100, 100);
        let a = ann(Kind::Obscure {
            rect: rect(80.0, 10.0, 60.0, 16.0),
            mode: ObscureMode::Pixelate,
            strength: 8,
        });
        let (patch, region) = obscure_patch(&img, &a).unwrap();
        assert_eq!(
            region,
            rect(80.0, 10.0, 20.0, 16.0),
            "clipped at the image edge"
        );
        assert_eq!((patch.width, patch.height), (20, 16));
        let distinct: std::collections::HashSet<u8> =
            patch.rgba.chunks_exact(4).map(|p| p[0]).collect();
        assert!(
            distinct.iter().all(|v| (120..=135).contains(v)),
            "stripes should average out: {distinct:?}"
        );
        assert!(obscure_patch(
            &img,
            &ann(Kind::Obscure {
                rect: rect(500.0, 0.0, 5.0, 5.0),
                mode: ObscureMode::Blur,
                strength: 4
            })
        )
        .is_none());
        assert!(obscure_patch(
            &img,
            &ann(Kind::Line {
                from: Point::new(0.0, 0.0),
                to: Point::new(1.0, 1.0)
            })
        )
        .is_none());
    }

    #[test]
    fn drafting_ignores_tiny_drags_and_builds_the_right_kinds() {
        let p = Props::default();
        let (a, b) = (Point::new(10.0, 10.0), Point::new(60.0, 40.0));
        assert!(drafted(Tool::Rectangle, a, Point::new(10.5, 10.5), &[], &p).is_none());
        assert!(
            matches!(drafted(Tool::Rectangle, b, a, &[], &p).unwrap().kind, Kind::Rectangle { rect } if rect == RectF { x: 10.0, y: 10.0, w: 50.0, h: 30.0 })
        );
        assert!(
            matches!(drafted(Tool::Arrow, a, b, &[], &p).unwrap().kind, Kind::Arrow { from, to } if from == a && to == b)
        );
        assert!(matches!(
            drafted(Tool::Blur, a, b, &[], &p).unwrap().kind,
            Kind::Obscure { .. }
        ));
        let hl = drafted(Tool::Highlighter, a, b, &[a, b], &p).unwrap();
        assert_eq!(hl.style.color.0[3], props::HIGHLIGHT_ALPHA);
        assert!(drafted(Tool::Text, a, b, &[], &p).is_none());
        assert!(drafted(Tool::Select, a, b, &[], &p).is_none());
    }

    #[test]
    fn the_gui_help_lists_copy_save_and_cancel_with_their_keys() {
        let rows = shortcut_rows();
        let keys = |what: &str| {
            rows.iter()
                .find(|(w, _)| w == what)
                .map(|(_, k)| k.as_str())
        };
        assert_eq!(keys("Copy to clipboard"), Some("Enter / Ctrl+C"));
        assert_eq!(keys("Save the screenshot in a file"), Some("Ctrl+S"));
        assert_eq!(keys("Cancel"), Some("Esc"));
        assert_eq!(keys("Select a region"), Some("Drag"));
    }

    #[test]
    fn the_gui_help_names_the_key_of_every_tool() {
        let rows = shortcut_rows();
        let (_, tools) = rows.iter().find(|(w, _)| w == "Choose a tool").unwrap();
        for t in Tool::ALL {
            assert!(
                tools.split(' ').any(|k| k == t.key().name()),
                "{:?}",
                t.key()
            );
        }
    }
}
