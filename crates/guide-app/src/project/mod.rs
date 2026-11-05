//! The guide window: a list of steps on the left, the selected step on the right.
//! Edits are saved to the project folder automatically a moment after you stop.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use crate::dialog;
use crate::i18n::{tr, tr_fmt, tr_n};
use crate::theme;
use eframe::egui::{self, Color32, Key, Pos2, Rect, Sense, TextureHandle, Vec2};
use guide_capture::Screenshot;
use guide_core::{
    DescriptionPlacement, Document, ImageAlign, ImageScaling, ItemId, NewItem, Project,
    ProjectKind, StepLayout,
};
use guide_export::{
    planned_files, Cancel, ExportMode, ExportOptions, ExportReport, NameStyle, Progress,
};
use guide_render::{
    compose::{compute_diff_layout, compute_layout, render_layout, ComposedLayout},
    render,
    resample::resample,
    Image,
};
use guide_storage::{ImageFormat, Issue, IssueKind, ProjectStore};

/// Edits are written this long after the last change.
const AUTOSAVE_DELAY: Duration = Duration::from_millis(1500);
const ROW_HEIGHT: f32 = 68.0;
const THUMB: Vec2 = Vec2::new(96.0, 60.0);
/// Rendered previews are kept at most this big (longest side).
const PREVIEW_SIDE: u32 = 960;
const MAX_JOBS: usize = 2;
const MAX_CACHED_PREVIEWS: usize = 64;
/// The quick composition preview is drawn about this wide (layout pixels are scaled to fit).
const PREVIEW_WIDTH: f32 = 1000.0;
/// Annotated step images kept for the quick preview are at most this wide.
const CACHE_SIDE: u32 = 1600;
/// A change must be this old before the composition preview is redrawn.
const COMP_DEBOUNCE: Duration = Duration::from_millis(250);

/// What the guide window asks the application to do.
pub enum ProjectCommand {
    EditStep(ItemId),
    /// The toolbar's buttons that belong to the application, not the project.
    Capture,
    CancelCapture,
    Close,
    OpenSettings,
}

/// What the application lends the toolbar: its capture state and the "add captures" switch.
pub struct ProjectBar<'a> {
    pub auto_add: &'a mut bool,
    pub capture_options_open: &'a mut bool,
    pub can_capture: bool,
    pub capturing: bool,
    pub countdown: Option<u64>,
    pub app_status: &'a str,
}

/// The tabs of the inspector on the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InspectorTab {
    Guide,
    Step,
    Layout,
    Numbering,
}

/// The text fields of the guide window that have a "Browse..." button.
#[derive(Clone, Copy)]
enum PathField {
    ImportImage,
    /// The rail's "Import image" button: the chosen file is imported at once.
    ImportNow,
    ExportFolder,
}

struct Preview {
    rev: u64,
    tex: TextureHandle,
    size: (u32, u32),
}

/// Annotated step images (shrunk) for the quick preview, keyed by step and revision.
type StepImageCache = Arc<Mutex<HashMap<(ItemId, u64), Arc<Image>>>>;

/// Settings, progress and result of exporting the guide.
struct ExportUi {
    mode: ExportMode,
    format: ImageFormat,
    dir: String,
    names: NameStyle,
    include_text: bool,
    replace: bool,
    running: Option<RunningExport>,
    progress: Option<Progress>,
    last: Option<ExportReport>,
    /// How many exports have been started in this session.
    runs: u32,
}

struct RunningExport {
    rx: mpsc::Receiver<ExportMsg>,
    cancel: Cancel,
}

enum ExportMsg {
    Progress(Progress),
    Done(ExportReport),
}

/// Examples of the two file name styles, with the extension of the chosen format.
fn file_name_examples(format: ImageFormat) -> (String, String) {
    let ext = format.extension();
    (format!("01-title.{ext}"), format!("01.{ext}")) // i18n-ok
}

impl Drop for ProjectView {
    fn drop(&mut self) {
        // Closing the project (or the window) must not leave an export writing on its own.
        if let Some(run) = &self.export.running {
            run.cancel.cancel();
        }
    }
}

/// The composition preview that was last drawn.
struct CompResult {
    rev: u64,
    final_res: bool,
    layout: ComposedLayout,
    tex: TextureHandle,
    /// Texture pixels per layout pixel.
    scale: f32,
}

struct CompMsg {
    rev: u64,
    final_res: bool,
    result: Result<(ComposedLayout, Screenshot, f32), String>,
}

struct JobResult {
    id: ItemId,
    rev: u64,
    image: Result<Screenshot, String>,
}

pub struct ProjectView {
    store: ProjectStore,
    project: Project,
    issues: Vec<Issue>,
    selected: Option<ItemId>,
    previews: HashMap<ItemId, Preview>,
    /// Bumped whenever what a step looks like changes (annotations, image).
    revs: HashMap<ItemId, u64>,
    in_flight: HashSet<ItemId>,
    failed: HashMap<ItemId, (u64, String)>,
    jobs_tx: mpsc::Sender<JobResult>,
    jobs_rx: mpsc::Receiver<JobResult>,
    wanted: Vec<ItemId>,
    dragging: Option<ItemId>,
    confirm_remove: Option<ItemId>,
    import_path: String,
    /// A file or folder dialog opened from a "Browse..." button.
    browsing: dialog::Pending<PathField>,
    dirty_since: Option<Instant>,
    status: String,
    /// Why the last save failed; shown as a banner until a save works.
    save_error: Option<String>,
    /// Desktop notification after an explicit save or an export.
    notify: crate::notify::NotifyCfg,
    report_path: Option<PathBuf>,
    last_report: String,
    /// Where things were drawn last frame (window-relative points); feeds the test report.
    layout: Layout,
    scroll_to_selected: bool,
    comp: Option<CompResult>,
    comp_error: Option<(u64, bool, String)>,
    /// Bumped by every change that can alter the combined image.
    comp_rev: u64,
    comp_changed_at: Instant,
    comp_running: bool,
    comp_tx: mpsc::Sender<CompMsg>,
    comp_rx: mpsc::Receiver<CompMsg>,
    step_cache: StepImageCache,
    final_preview: bool,
    /// Screen points per layout pixel in the composition preview.
    zoom: f32,
    auto_fit: bool,
    left_open: bool,
    right_open: bool,
    inspector_tab: InspectorTab,
    panels_decided: bool,
    preview_scrolled_to: Option<ItemId>,
    export: ExportUi,
}

#[derive(Default)]
struct Layout {
    rows: Vec<(ItemId, Rect)>,
    title: Option<Rect>,
    description: Option<Rect>,
    included: Option<Rect>,
    max_width: Option<Rect>,
    show_descriptions: Option<Rect>,
    /// Step boxes in the composition preview, and the preview picture itself.
    boxes: Vec<(ItemId, Rect)>,
    preview: Option<Rect>,
    toggle_left: Option<Rect>,
    toggle_right: Option<Rect>,
    toggle_final: Option<Rect>,
    export_button: Option<Rect>,
    export_separate: Option<Rect>,
    export_combined: Option<Rect>,
    /// Inspector tabs: guide, step, layout, numbering.
    tabs: Vec<Rect>,
}

/// A folder name for a new guide: lower-case letters, digits and single dashes.
pub fn slug(title: &str) -> String {
    guide_core::slug(title, 60, "guide")
}

/// `<parent>/<slug>.<project extension>`, or `-2`, `-3`... if that already exists.
pub fn new_project_dir(parent: &Path, title: &str) -> PathBuf {
    let base = slug(title);
    (1u32..)
        .map(|n| {
            if n == 1 {
                format!("{base}.{}", guide_core::PROJECT_EXTENSION)
            } else {
                format!("{base}-{n}.{}", guide_core::PROJECT_EXTENSION)
            }
        })
        .map(|name| parent.join(name))
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

impl ProjectView {
    fn from_parts(
        store: ProjectStore,
        project: Project,
        issues: Vec<Issue>,
        status: String,
    ) -> Self {
        let export_dir = store.dir().join("exports").to_string_lossy().into_owned();
        let (jobs_tx, jobs_rx) = mpsc::channel();
        let (comp_tx, comp_rx) = mpsc::channel();
        let selected = project.items.first().map(|s| s.id);
        Self {
            store,
            project,
            issues,
            selected,
            previews: HashMap::new(),
            revs: HashMap::new(),
            in_flight: HashSet::new(),
            failed: HashMap::new(),
            jobs_tx,
            jobs_rx,
            wanted: Vec::new(),
            dragging: None,
            confirm_remove: None,
            import_path: String::new(),
            browsing: dialog::Pending::default(),
            dirty_since: None,
            status,
            save_error: None,
            notify: Default::default(),
            report_path: std::env::var_os("SNAPFORGE_TEST_GUIDE_REPORT").map(PathBuf::from),
            last_report: String::new(),
            layout: Layout::default(),
            scroll_to_selected: false,
            comp: None,
            comp_error: None,
            comp_rev: 1,
            comp_changed_at: Instant::now(),
            comp_running: false,
            comp_tx,
            comp_rx,
            step_cache: Arc::new(Mutex::new(HashMap::new())),
            final_preview: false,
            zoom: 0.5,
            auto_fit: true,
            left_open: true,
            right_open: true,
            inspector_tab: InspectorTab::Guide,
            panels_decided: false,
            preview_scrolled_to: None,
            export: ExportUi {
                mode: ExportMode::Separate,
                format: ImageFormat::Png,
                dir: export_dir,
                names: NameStyle::NumberAndTitle,
                include_text: false,
                replace: false,
                running: None,
                progress: None,
                last: None,
                runs: 0,
            },
        }
    }

    /// Creates `<parent>/<slug of title>.<project extension>` holding an empty project of `kind`.
    pub fn create(parent: &Path, title: &str, kind: ProjectKind) -> Result<Self, String> {
        let title = if !title.trim().is_empty() {
            title.trim()
        } else {
            match kind {
                ProjectKind::Plain => tr("Untitled project"),
                ProjectKind::Guide(_) => tr("Untitled guide"),
                ProjectKind::Diff(_) => tr("Untitled diff"),
            }
        };
        let dir = new_project_dir(parent, title);
        let project = Project::new(title, kind);
        let store = ProjectStore::create(&dir, &project).map_err(|e| e.to_string())?;
        Ok(Self::from_parts(
            store,
            project,
            Vec::new(),
            tr_fmt("Created {folder}", &[("folder", &dir.display())]),
        ))
    }

    pub fn open(dir: &Path) -> Result<Self, String> {
        let o = ProjectStore::open(dir).map_err(|e| e.to_string())?;
        let mut status = o.notice.unwrap_or_default();
        if !o.issues.is_empty() {
            let n = o.issues.len();
            let note = tr_n(
                "{n} step has a problem with its screenshot; see below.",
                "{n} steps have problems with their screenshots; see below.",
                n,
                &[],
            );
            status = if status.is_empty() {
                note
            } else {
                format!("{status} {note}")
            };
        }
        Ok(Self::from_parts(o.store, o.project, o.issues, status))
    }

    /// Shows a message in the guide window's own status line.
    pub fn notify(&mut self, message: String) {
        self.status = message;
    }

    /// The name to show: the project's own, or a stand-in while the field is empty.
    pub fn title(&self) -> String {
        let name = self.project.name.trim();
        if !name.is_empty() {
            name.to_string()
        } else {
            self.untitled().to_string()
        }
    }

    fn untitled(&self) -> &'static str {
        if self.project.is_guide() {
            tr("Untitled guide")
        } else if self.project.is_diff() {
            tr("Untitled diff")
        } else {
            tr("Untitled project")
        }
    }

    /// Whether this project is drawn as one composed image (a guide or a diff).
    fn composes(&self) -> bool {
        self.project.is_guide() || self.project.is_diff()
    }

    pub fn dir(&self) -> &Path {
        self.store.dir()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    /// Every change to the guide goes through here: it must be saved, and the combined
    /// image has to be drawn again.
    fn mark_dirty(&mut self) {
        let now = Instant::now();
        self.dirty_since = Some(now);
        self.comp_rev += 1;
        self.comp_changed_at = now;
    }

    fn rev(&self, id: ItemId) -> u64 {
        self.revs.get(&id).copied().unwrap_or(0)
    }

    fn bump(&mut self, id: ItemId) {
        *self.revs.entry(id).or_insert(0) += 1;
        self.failed.remove(&id);
    }

    // ---- saving -------------------------------------------------------------------

    /// Set by the window around the guide whenever the notification settings change.
    pub fn set_notify(&mut self, cfg: crate::notify::NotifyCfg) {
        self.notify = cfg;
    }

    fn announce(&self, message: &str) {
        crate::notify::send(&self.notify, guide_core::APP_NAME, message);
    }

    /// A save the user asked for (button, Ctrl+S, "Try again"); unlike autosave it is announced.
    fn save_by_request(&mut self) {
        if self.save_now() {
            self.announce(tr("Project saved"));
        }
    }

    /// Writes the guide now. Returns `false` (and shows why) if it could not be saved.
    pub fn save_now(&mut self) -> bool {
        match self.store.save(&self.project) {
            Ok(()) => {
                self.dirty_since = None;
                self.save_error = None;
                self.status = tr("Saved").into();
                true
            }
            Err(e) => {
                self.status = tr_fmt("Could not save the project: {error}", &[("error", &e)]);
                self.save_error = Some(e.to_string());
                // Try again after the usual delay instead of on every frame.
                if self.dirty_since.is_some() {
                    self.dirty_since = Some(std::time::Instant::now());
                }
                false
            }
        }
    }

    /// Saves and deletes screenshots no step uses any more. Only done when leaving the
    /// guide, so a step removed by mistake is never lost the moment it is removed.
    /// Returns `false` if the guide could not be saved, in which case nothing is deleted.
    pub fn finish(&mut self) -> bool {
        if !self.save_now() {
            return false;
        }
        // An export works on a copy of the project and reads the screenshots; deleting the ones
        // this project no longer uses would pull them from under it.
        if !self.stop_export(Duration::from_secs(10)) {
            self.status =
                tr("Saved. Unused images were kept because an export is still running.").into();
            return true;
        }
        match self.store.prune_unreferenced_assets(&self.project) {
            Ok(_) => true,
            Err(e) => {
                self.status = tr_fmt(
                    "Saved, but could not clean up unused images: {error}",
                    &[("error", &e)],
                );
                true
            }
        }
    }

    /// Asks a running export to stop and waits up to `wait` for it to finish, so nothing it
    /// reads is deleted from under it. `true` when no export is running any more.
    fn stop_export(&mut self, wait: Duration) -> bool {
        let Some(run) = &self.export.running else {
            return true;
        };
        run.cancel.cancel();
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match run.rx.recv_timeout(left) {
                Ok(ExportMsg::Done(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.export.running = None;
                    self.export.progress = None;
                    return true;
                }
                Ok(ExportMsg::Progress(_)) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => return false,
            }
        }
    }

    // ---- editing the guide --------------------------------------------------------------

    fn insert_position(&self) -> usize {
        self.selected
            .and_then(|id| self.project.index_of(id))
            .map_or(usize::MAX, |i| i + 1)
    }

    /// A diff holds a previous and a post screenshot, no more; says so when it is full.
    fn diff_is_full(&mut self) -> bool {
        if self.project.can_add_item() {
            return false;
        }
        self.status =
            tr("A diff has a previous and a post screenshot; remove one to add another").into();
        true
    }

    fn add_step_with_image(&mut self, rel: String, title: String) {
        if self.diff_is_full() {
            return;
        }
        let index = self.insert_position();
        // A diff names its screenshots by role, and captions them the same way until edited.
        let (title, caption) = if self.project.is_diff() {
            let role = if self.project.items.is_empty() {
                tr("Before")
            } else {
                tr("After")
            };
            (role.to_string(), role.to_string())
        } else {
            (title, String::new())
        };
        let id = self.project.add_item(
            index,
            NewItem {
                title,
                description: String::new(),
                caption,
                image: rel,
            },
        );
        self.selected = Some(id);
        self.scroll_to_selected = true;
        self.mark_dirty();
    }

    /// Adds a screenshot as a new step after the selected one (or at the end).
    pub fn add_capture(&mut self, shot: &Screenshot) {
        // Not "Step N": a step inserted in the middle would get a misleading number.
        if self.diff_is_full() {
            return;
        }
        let title = tr("New step").to_string();
        match self
            .store
            .add_asset_rgba(shot.width, shot.height, &shot.rgba)
        {
            Ok(rel) => {
                self.add_step_with_image(rel, title);
                self.status = tr("Added the capture as a new step").into();
            }
            Err(e) => {
                self.status = tr_fmt("Could not add the screenshot: {error}", &[("error", &e)])
            }
        }
    }

    /// Like [`Self::add_capture`], and when `click` (a position in the
    /// screenshot) is given, marks it with a numbered marker annotation. The
    /// stored screenshot itself is never touched.
    pub fn add_capture_with_marker(
        &mut self,
        shot: &Screenshot,
        click: Option<(i32, i32)>,
        style: guide_core::Style,
    ) {
        let before = self.project.items.len();
        self.add_capture(shot);
        let (Some((x, y)), true, Some(id)) =
            (click, self.project.items.len() > before, self.selected)
        else {
            return;
        };
        let radius = (shot.width.min(shot.height) as f32 / 60.0).max(14.0);
        let mut editor = guide_core::Editor::new(Document::default());
        editor.add_marker(guide_core::Point::new(x as f32, y as f32), radius, style);
        self.set_annotations(id, editor.document().clone());
    }

    /// Puts the path chosen in a "Browse..." dialog into the field it was opened for.
    fn take_browsed_path(&mut self) {
        if let Some((field, path)) = self.browsing.take_chosen() {
            let path = path.to_string_lossy().into_owned();
            match field {
                PathField::ImportImage => self.import_path = path,
                PathField::ImportNow => {
                    self.import_path = path;
                    self.import();
                }
                PathField::ExportFolder => self.export.dir = path,
            }
        }
    }

    fn import(&mut self) {
        if self.diff_is_full() {
            return;
        }
        let path = PathBuf::from(self.import_path.trim());
        match self.store.import_image(&path) {
            Ok((rel, w, h)) => {
                let title = path.file_stem().map_or_else(
                    || {
                        tr_fmt(
                            "Step {number}",
                            &[("number", &(self.project.items.len() + 1))],
                        )
                    },
                    |s| s.to_string_lossy().into_owned(),
                );
                self.add_step_with_image(rel, title);
                self.status = tr_fmt("Imported {w}x{h} image", &[("w", &w), ("h", &h)]);
                self.import_path.clear();
            }
            Err(e) => self.status = tr_fmt("Could not import: {error}", &[("error", &e)]),
        }
    }

    fn select(&mut self, id: Option<ItemId>) {
        self.selected = id;
    }

    fn select_offset(&mut self, delta: isize) {
        let n = self.project.items.len() as isize;
        if n == 0 {
            return;
        }
        let cur = self
            .selected
            .and_then(|id| self.project.index_of(id))
            .map_or(-1, |i| i as isize);
        let next = (cur + delta).clamp(0, n - 1) as usize;
        self.selected = Some(self.project.items[next].id);
        self.scroll_to_selected = true;
    }

    fn move_selected(&mut self, delta: isize) {
        let Some(id) = self.selected else { return };
        let Some(i) = self.project.index_of(id) else {
            return;
        };
        let to = (i as isize + delta).clamp(0, self.project.items.len() as isize - 1) as usize;
        if to != i && self.project.move_item(id, to) {
            self.scroll_to_selected = true;
            self.mark_dirty();
        }
    }

    fn duplicate_selected(&mut self) {
        let copy = self.selected.and_then(|id| {
            self.project
                .duplicate_item(id, |title| tr_fmt("{title} (copy)", &[("title", &title)]))
        });
        if let Some(copy) = copy {
            self.selected = Some(copy);
            self.scroll_to_selected = true;
            self.mark_dirty();
        }
    }

    fn remove(&mut self, id: ItemId) {
        let Some(index) = self.project.index_of(id) else {
            return;
        };
        self.project.remove_item(id);
        self.previews.remove(&id);
        self.issues.retain(|i| i.item != id);
        // Select the neighbour that took its place.
        self.selected = self
            .project
            .items
            .get(index)
            .or_else(|| self.project.items.last())
            .map(|s| s.id);
        self.mark_dirty();
    }

    /// Stores annotations edited in the annotation editor.
    pub fn set_annotations(&mut self, id: ItemId, doc: Document) {
        if let Some(step) = self.project.item_mut(id) {
            if step.annotations != doc {
                step.annotations = doc;
                self.bump(id);
                self.mark_dirty();
            }
        }
    }

    /// The selected step's original screenshot, for the annotation editor.
    pub fn load_for_editing(&self, id: ItemId) -> Result<(Screenshot, Document), String> {
        let step = self
            .project
            .item(id)
            .ok_or_else(|| tr("that step no longer exists").to_string())?;
        let o = self
            .store
            .load_asset(&step.image)
            .map_err(|e| e.to_string())?;
        Ok((
            Screenshot {
                width: o.width,
                height: o.height,
                rgba: o.rgba,
            },
            step.annotations.clone(),
        ))
    }

    // ---- previews -----------------------------------------------------------------------

    fn issue_for(&self, id: ItemId) -> Option<&Issue> {
        self.issues
            .iter()
            .find(|i| i.item == id && i.kind != IssueKind::InvalidAnnotations)
    }

    fn pump_jobs(&mut self, ctx: &egui::Context) {
        while let Ok(r) = self.jobs_rx.try_recv() {
            self.in_flight.remove(&r.id);
            if self.project.item(r.id).is_none() {
                continue;
            }
            match r.image {
                Ok(shot) => {
                    let size = (shot.width, shot.height);
                    let tex = crate::app::texture_for(ctx, "guide-preview", &shot);
                    self.previews.insert(
                        r.id,
                        Preview {
                            rev: r.rev,
                            tex,
                            size,
                        },
                    );
                }
                Err(e) => {
                    self.failed.insert(r.id, (r.rev, e));
                }
            }
        }
        let mut wanted = std::mem::take(&mut self.wanted);
        wanted.extend(self.selected);
        for id in &wanted {
            if self.in_flight.len() >= MAX_JOBS {
                break;
            }
            let rev = self.rev(*id);
            let fresh = self.previews.get(id).is_some_and(|p| p.rev == rev);
            let failed = self.failed.get(id).is_some_and(|(r, _)| *r == rev);
            if fresh || failed || self.in_flight.contains(id) || self.issue_for(*id).is_some() {
                continue;
            }
            let Some(step) = self.project.item(*id).cloned() else {
                continue;
            };
            self.in_flight.insert(*id);
            let (store, tx, ctx2, id) =
                (self.store.clone(), self.jobs_tx.clone(), ctx.clone(), *id);
            thread::spawn(move || {
                let image = (|| -> Result<Screenshot, String> {
                    let o = store.load_asset(&step.image).map_err(|e| e.to_string())?;
                    let original = Image {
                        width: o.width,
                        height: o.height,
                        rgba: o.rgba,
                    };
                    let out = render(&original, &step.annotations).map_err(|e| e.to_string())?;
                    let shot = Screenshot {
                        width: out.width,
                        height: out.height,
                        rgba: out.rgba,
                    };
                    Ok(shot.scaled_to_fit(PREVIEW_SIDE).unwrap_or(shot))
                })();
                let _ = tx.send(JobResult { id, rev, image });
                ctx2.request_repaint();
            });
        }
        if !self.in_flight.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        // Keep memory bounded on very long guides: drop previews nobody is looking at.
        if self.previews.len() > MAX_CACHED_PREVIEWS {
            let keep: HashSet<ItemId> = wanted.into_iter().collect();
            let drop: Vec<ItemId> = self
                .previews
                .keys()
                .filter(|id| !keep.contains(id))
                .copied()
                .collect();
            for id in drop
                .into_iter()
                .take(self.previews.len() - MAX_CACHED_PREVIEWS / 2)
            {
                self.previews.remove(&id);
            }
        }
    }

    // ---- frame ---------------------------------------------------------------------------

    pub fn show(&mut self, ctx: &egui::Context, bar: &mut ProjectBar) -> Vec<ProjectCommand> {
        let mut commands = Vec::new();
        self.pump_jobs(ctx);
        self.pump_composition(ctx);
        self.pump_export(ctx);
        self.take_browsed_path();
        self.layout = Layout::default();
        self.handle_keys(ctx, &mut commands);

        if !self.panels_decided {
            // On a narrow window start with the properties folded away.
            self.right_open = properties_open_by_default(ctx.screen_rect().width());
            self.panels_decided = true;
        }
        self.toolbar(ctx, bar, &mut commands);
        self.save_banner(ctx);
        if self.left_open {
            egui::SidePanel::left("guide_steps")
                .frame(panel_frame(ctx, 16))
                .default_width(260.0)
                .min_width(180.0)
                .show(ctx, |ui| self.step_list(ui, ctx));
        }
        if self.right_open {
            egui::SidePanel::right("guide_properties")
                .frame(panel_frame(ctx, 0))
                .default_width(340.0)
                .min_width(220.0)
                .show(ctx, |ui| self.properties(ui, ctx, &mut commands));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).inner_margin(egui::Margin::same(0)))
            .show(ctx, |ui| self.preview_panel(ui, ctx));
        self.remove_dialog(ctx);

        if let Some(since) = self.dirty_since {
            if since.elapsed() >= AUTOSAVE_DELAY {
                self.save_now();
                // The "unsaved changes" label was drawn before the save; redraw it.
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(AUTOSAVE_DELAY);
            }
        }
        self.write_report(ctx);
        commands
    }

    /// The bar across the top: capture, the project's name and save state, Save, Export, Close.
    fn toolbar(
        &mut self,
        ctx: &egui::Context,
        bar: &mut ProjectBar,
        commands: &mut Vec<ProjectCommand>,
    ) {
        let pal = theme::palette(ctx);
        let frame =
            egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(16, 10));
        egui::TopBottomPanel::top("guide_toolbar") // i18n-ok
            .frame(frame)
            .show(ctx, |ui| {
                // The project's name and folder sit in the bar on a wide window, under it otherwise.
                let inline_title = ui.available_width() >= 1500.0;
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::Btn::new("⚙", theme::Emphasis::Secondary) // i18n-ok
                            .name(tr("Settings"))
                            .show(ui)
                            .on_hover_text(tr("Settings"))
                            .clicked()
                        {
                            commands.push(ProjectCommand::OpenSettings);
                        }
                        if theme::Btn::new(tr("Close project"), theme::Emphasis::Secondary)
                            .show(ui)
                            .clicked()
                        {
                            commands.push(ProjectCommand::Close);
                        }
                        if let Some(run) = &self.export.running {
                            if theme::Btn::new(tr("Cancel export"), theme::Emphasis::Secondary)
                                .show(ui)
                                .clicked()
                            {
                                run.cancel.cancel();
                            }
                            let p = self.export.progress.clone().unwrap_or(Progress {
                                done: 0,
                                total: 1,
                                message: String::new(),
                            });
                            ui.add(
                                egui::ProgressBar::new(p.done as f32 / p.total.max(1) as f32)
                                    .desired_width(150.0)
                                    .text(format!("{}/{}", p.done, p.total)),
                            )
                            .on_hover_text(&p.message);
                        } else {
                            let r = theme::Btn::new(tr("Export"), theme::Emphasis::Accent)
                                .chip("Ctrl E") // i18n-ok
                                .name(tr("Export (Ctrl+E)"))
                                .show(ui)
                                .on_hover_text(tr(
                                    "Write the project as images, as set in Export under Properties",
                                ));
                            self.layout.export_button = Some(r.rect);
                            if r.clicked() {
                                self.start_export(ctx);
                            }
                        }
                        if theme::Btn::new(tr("Save"), theme::Emphasis::Secondary)
                            .chip("Ctrl S") // i18n-ok
                            .name(tr("Save (Ctrl+S)"))
                            .show(ui)
                            .clicked()
                        {
                            self.save_by_request();
                        }
                        let dirty = self.is_dirty();
                        ui.label(if dirty {
                            tr("Unsaved changes")
                        } else {
                            tr("All changes saved")
                        });
                        let (dot, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(
                            dot.center(),
                            4.0,
                            if dirty { pal.accent } else { pal.success },
                        );
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled_ui(bar.can_capture, |ui| {
                                    theme::Btn::new(tr("Capture"), theme::Emphasis::Secondary)
                                        .chip("Ctrl N") // i18n-ok
                                        .name(tr("Capture (Ctrl+N)"))
                                        .show(ui)
                                })
                                .inner
                                .clicked()
                            {
                                commands.push(ProjectCommand::Capture);
                            }
                            if bar.capturing && ui.button(tr("Cancel (Esc)")).clicked() {
                                commands.push(ProjectCommand::CancelCapture);
                            }
                            if let Some(s) = bar.countdown {
                                ui.label(tr_fmt("Capturing in {seconds}...", &[("seconds", &s)]));
                            }
                            theme::checkbox(
                                ui,
                                &mut *bar.auto_add,
                                tr("Add captures to the project"),
                            )
                            .on_hover_text(tr("Every screenshot you take is added to the project"));
                            ui.toggle_value(&mut *bar.capture_options_open, tr("Capture and files"));
                            if inline_title {
                                ui.separator();
                                self.title_block(ui, &pal);
                            }
                        });
                    });
                });
                if !inline_title {
                    ui.add_space(4.0);
                    self.title_block(ui, &pal);
                }
                if !bar.app_status.is_empty() {
                    ui.label(egui::RichText::new(bar.app_status).color(pal.muted));
                }
            });
    }

    /// The project's name over its folder, cut off with an ellipsis when there is no room.
    fn title_block(&self, ui: &mut egui::Ui, pal: &theme::Palette) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(
                egui::Label::new(
                    egui::RichText::new(self.title())
                        .family(theme::strong_family())
                        .size(15.0),
                )
                .truncate(),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(self.dir().display().to_string())
                        .monospace()
                        .size(12.0)
                        .color(pal.muted),
                )
                .truncate(),
            );
        });
    }

    /// A persistent warning while the guide cannot be saved: a status line is too easy to miss.
    fn save_banner(&mut self, ctx: &egui::Context) {
        let Some(error) = self.save_error.clone() else {
            return;
        };
        let error_colour = crate::theme::palette(ctx).error;
        egui::TopBottomPanel::top("guide_save_banner").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    error_colour,
                    tr_fmt("Your changes are NOT saved: {error}", &[("error", &error)]),
                );
                if ui.button(tr("Try again")).clicked() {
                    self.save_by_request();
                }
                if ui.button(tr("Copy details")).clicked() {
                    ui.ctx().copy_text(error.clone());
                }
            });
        });
    }

    fn handle_keys(&mut self, ctx: &egui::Context, commands: &mut Vec<ProjectCommand>) {
        if self.confirm_remove.is_some() {
            return; // the dialog handles its own keys
        }
        let (cmd, alt) = ctx.input(|i| (i.modifiers.command, i.modifiers.alt));
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        // Ctrl shortcuts work while typing in a field (the text is already in the project);
        // the plain keys below belong to the field then.
        if cmd && pressed(Key::E) {
            self.start_export(ctx);
        } else if cmd && pressed(Key::S) {
            self.save_by_request();
        } else if cmd && pressed(Key::D) {
            self.duplicate_selected();
        } else if ctx.wants_keyboard_input() {
            // Typing: the remaining shortcuts would act on the steps instead of the text.
        } else if alt && pressed(Key::ArrowUp) {
            self.move_selected(-1);
        } else if alt && pressed(Key::ArrowDown) {
            self.move_selected(1);
        } else if pressed(Key::ArrowUp) {
            self.select_offset(-1);
        } else if pressed(Key::ArrowDown) {
            self.select_offset(1);
        } else if pressed(Key::Delete) {
            self.confirm_remove = self.selected;
        } else if pressed(Key::Enter) {
            if let Some(id) = self.selected {
                commands.push(ProjectCommand::EditStep(id));
            }
        }
    }

    fn remove_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_remove else {
            return;
        };
        let Some(step) = self.project.item(id) else {
            self.confirm_remove = None;
            return;
        };
        let name = if step.title.is_empty() {
            tr("this step").to_string()
        } else {
            format!("\"{}\"", step.title)
        };
        let shared = self
            .project
            .items
            .iter()
            .filter(|s| s.image == step.image)
            .count()
            > 1;
        let (mut yes, mut no) = (false, false);
        egui::Window::new(tr("Remove from the project"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(tr_fmt(
                    "Remove {name} from the project?",
                    &[("name", &name)],
                ));
                if shared {
                    ui.label(tr(
                        "Its screenshot is also used by another step and will be kept.",
                    ));
                }
                ui.horizontal(|ui| {
                    yes = ui.button(tr("Remove (Enter)")).clicked();
                    no = ui.button(tr("Cancel (Esc)")).clicked();
                });
            });
        let (enter, esc) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if yes || enter {
            self.confirm_remove = None;
            self.remove(id);
        } else if no || esc {
            self.confirm_remove = None;
        }
    }

    fn step_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = theme::palette(ui.ctx());
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(if self.project.is_guide() {
                    tr("Steps")
                } else {
                    tr("Screenshots")
                })
                .family(theme::strong_family())
                .size(18.0),
            );
            ui.label(
                egui::RichText::new(self.project.items.len().to_string())
                    .size(18.0)
                    .color(pal.muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::Btn::new(tr("Import image"), theme::Emphasis::Secondary)
                    .show(ui)
                    .clicked()
                {
                    self.browsing.start(
                        ui.ctx(),
                        PathField::ImportNow,
                        dialog::Kind::OpenImage,
                        &self.import_path,
                        tr("Choose an image to add as a step"),
                    );
                }
            });
        });
        ui.add_space(8.0);
        if self.project.items.is_empty() {
            let rect = ui
                .available_rect_before_wrap()
                .shrink2(egui::vec2(0.0, 4.0));
            ui.painter().extend(egui::Shape::dashed_line(
                &[
                    rect.left_top(),
                    rect.right_top(),
                    rect.right_bottom(),
                    rect.left_bottom(),
                    rect.left_top(),
                ],
                egui::Stroke::new(1.0, pal.border),
                6.0,
                5.0,
            ));
            ui.add_space(rect.height() / 3.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(if self.project.is_guide() {
                        tr("No steps yet.")
                    } else {
                        tr("No screenshots yet.")
                    })
                    .family(theme::strong_family()),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(tr("Capture a screenshot and press \"Add to project\", turn on \"Add captures to the project\", or import an image."))
                        .color(pal.muted),
                );
            });
            return;
        }
        let n = self.project.items.len();
        let pitch = ROW_HEIGHT + ui.spacing().item_spacing.y;
        let selected_index = self.selected.and_then(|id| self.project.index_of(id));
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if std::mem::take(&mut self.scroll_to_selected) {
            if let Some(i) = selected_index {
                area = area.vertical_scroll_offset((i as f32 * pitch - 2.0 * pitch).max(0.0));
            }
        }
        area.show_rows(ui, ROW_HEIGHT, n, |ui, range| {
            let first_top = ui.cursor().top();
            let width = ui.available_width();
            let mut drop_gap: Option<usize> = None;
            for i in range.clone() {
                let id = self.project.items[i].id;
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::click_and_drag());
                let selected = self.selected == Some(id);
                resp.widget_info(|| {
                    let step = &self.project.items[i];
                    let title = if step.title.is_empty() {
                        tr("(untitled)")
                    } else {
                        step.title.as_str()
                    };
                    let mut label = tr_fmt(
                        "Step {number}: {title}",
                        &[("number", &(i + 1)), ("title", &title)],
                    );
                    if !step.included {
                        label = format!("{label}, {}", tr("excluded"));
                    }
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        selected,
                        label,
                    )
                });
                self.wanted.push(id);
                self.layout.rows.push((id, rect));
                self.paint_row(ui, i, rect, resp.hovered());
                if resp.drag_started() {
                    self.dragging = Some(id);
                    self.select(Some(id));
                } else if resp.clicked() {
                    self.select(Some(id));
                }
            }
            if let Some(dragged) = self.dragging {
                if let Some(p) = ctx.pointer_interact_pos() {
                    // The gap the step would be dropped into: 0..=n, counted in whole rows.
                    let slot = range.start as f32 + (p.y - first_top) / pitch;
                    let gap = (slot + 0.5).floor().clamp(0.0, n as f32) as usize;
                    drop_gap = Some(gap);
                    let y = first_top + (gap as f32 - range.start as f32) * pitch
                        - ui.spacing().item_spacing.y / 2.0;
                    ui.painter().hline(
                        ui.max_rect().x_range(),
                        y,
                        egui::Stroke::new(3.0, crate::theme::palette(ui.ctx()).accent),
                    );
                    // Scroll the list when the pointer is near its top or bottom edge.
                    let clip = ui.clip_rect();
                    if p.y < clip.top() + 24.0 {
                        ui.scroll_with_delta(Vec2::new(0.0, 12.0));
                    } else if p.y > clip.bottom() - 24.0 {
                        ui.scroll_with_delta(Vec2::new(0.0, -12.0));
                    }
                    ctx.request_repaint();
                }
                if ctx.input(|i| i.pointer.any_released()) {
                    if let (Some(gap), Some(from)) = (drop_gap, self.project.index_of(dragged)) {
                        // `move_step` takes the final index, after the step is taken out.
                        let to = if gap > from { gap - 1 } else { gap };
                        if to != from && self.project.move_item(dragged, to) {
                            self.mark_dirty();
                        }
                    }
                    self.dragging = None;
                }
            }
        });
        if self.dragging.is_some() && ctx.input(|i| !i.pointer.any_down()) {
            self.dragging = None;
        }
    }

    fn paint_row(&self, ui: &egui::Ui, index: usize, rect: Rect, hovered: bool) {
        let step = &self.project.items[index];
        let selected = self.selected == Some(step.id);
        let p = ui.painter();
        let pal = crate::theme::palette(ui.ctx());
        let fill = if selected {
            pal.row_selected
        } else if hovered {
            pal.row_hover
        } else {
            pal.row
        };
        p.rect_filled(rect, 4.0, fill);
        let thumb = Rect::from_min_size(rect.min + Vec2::new(6.0, 4.0), THUMB);
        p.rect_filled(thumb, 2.0, pal.thumb_background);
        if let Some(pre) = self.previews.get(&step.id) {
            let scale = (THUMB.x / pre.size.0 as f32).min(THUMB.y / pre.size.1 as f32);
            let size = Vec2::new(pre.size.0 as f32, pre.size.1 as f32) * scale;
            let img = Rect::from_center_size(thumb.center(), size);
            p.image(
                pre.tex.id(),
                img,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let broken = self.issue_for(step.id).is_some();
        let text_x = thumb.right() + 8.0;
        // A selected row draws all its text in one colour chosen for that background.
        let (color, secondary) = if selected {
            (pal.row_selected_text, pal.row_selected_text)
        } else if step.included {
            (pal.text, pal.muted)
        } else {
            (pal.muted, pal.muted)
        };
        p.text(
            Pos2::new(text_x, rect.top() + 8.0),
            egui::Align2::LEFT_TOP,
            format!("{}.", index + 1),
            egui::FontId::proportional(14.0),
            secondary,
        );
        let title = if step.title.is_empty() {
            tr("(untitled)")
        } else {
            step.title.as_str()
        };
        p.with_clip_rect(Rect::from_min_max(
            Pos2::new(text_x + 22.0, rect.top()),
            rect.max,
        ))
        .text(
            Pos2::new(text_x + 22.0, rect.top() + 8.0),
            egui::Align2::LEFT_TOP,
            title,
            egui::FontId::proportional(14.0),
            color,
        );
        let mut flags = Vec::new();
        if !step.included {
            flags.push(tr("excluded"));
        }
        if broken {
            flags.push(tr("(!) image problem"));
        }
        if !flags.is_empty() {
            p.text(
                Pos2::new(text_x, rect.top() + 32.0),
                egui::Align2::LEFT_TOP,
                flags.join(", "),
                egui::FontId::proportional(12.0),
                if broken && !selected {
                    pal.error
                } else {
                    secondary
                },
            );
        }
    }

    fn issue_list(&mut self, ui: &mut egui::Ui) {
        if self.issues.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(tr_fmt(
            "Problems ({count})",
            &[("count", &self.issues.len())],
        ))
        .default_open(true)
        .show(ui, |ui| {
            for issue in &self.issues {
                ui.colored_label(crate::theme::palette(ui.ctx()).error, &issue.message);
            }
        });
    }

    /// Test hook: where things are on screen, so end-to-end tests can click them.
    fn write_report(&mut self, ctx: &egui::Context) {
        let Some(path) = &self.report_path else {
            return;
        };
        let ppp = ctx.pixels_per_point();
        let (ox, oy) = ctx
            .input(|i| i.viewport().inner_rect)
            .map_or((0.0, 0.0), |r| (r.min.x * ppp, r.min.y * ppp));
        let rect = |r: &Rect| {
            format!(
                "[{:.1},{:.1},{:.1},{:.1}]",
                ox + r.min.x * ppp,
                oy + r.min.y * ppp,
                r.width() * ppp,
                r.height() * ppp
            )
        };
        let opt = |r: &Option<Rect>| r.as_ref().map_or_else(|| "null".to_string(), rect);
        let rows: Vec<String> = self
            .layout
            .rows
            .iter()
            .map(|(id, r)| format!("{{\"id\":{},\"rect\":{}}}", id.0, rect(r)))
            .collect();
        let order: Vec<String> = self
            .project
            .items
            .iter()
            .map(|s| s.id.0.to_string())
            .collect();
        let export_json = format!(
                "{{\"mode\":{:?},\"state\":{:?},\"runs\":{},\"written\":{},\"problems\":{},\"cancelled\":{},\"button\":{},\"separate\":{},\"combined\":{}}}",
                format!("{:?}", self.export.mode),
                if self.export.running.is_some() { "running" } else if self.export.last.is_some() { "done" } else { "idle" },
                self.export.runs,
                self.export.last.as_ref().map_or(0, |r| r.written.len()),
                self.export.last.as_ref().map_or(0, |r| r.problems.len()),
                self.export.last.as_ref().is_some_and(|r| r.cancelled),
                opt(&self.layout.export_button),
                opt(&self.layout.export_separate),
                opt(&self.layout.export_combined),
        );
        let report = format!(
            "{{\"order\":[{}],\"rows\":[{}],\"selected\":{},\"dirty\":{},\"issues\":{},\"confirm_remove\":{},\"title\":{},\"description\":{},\"included\":{},\"max_width\":{},\"show_descriptions\":{},\"preview\":{},\"preview_state\":{:?},\"preview_size\":{},\"preview_final\":{},\"preview_scale\":{:.4},\"toggle_left\":{},\"toggle_right\":{},\"toggle_final\":{},\"export\":{},\"tabs\":[{}],\"boxes\":[{}],\"zoom\":{:.4},\"left_open\":{},\"right_open\":{},\"status\":{:?},\"ppp\":{},\"native_ppp\":{},\"zoom_factor\":{}}}",
            order.join(","),
            rows.join(","),
            self.selected.map_or(0, |i| i.0),
            self.is_dirty(),
            self.issues.len(),
            self.confirm_remove.is_some(),
            opt(&self.layout.title),
            opt(&self.layout.description),
            opt(&self.layout.included),
            opt(&self.layout.max_width),
            opt(&self.layout.show_descriptions),
            opt(&self.layout.preview),
            self.preview_state(),
            self.comp.as_ref().map_or_else(
                || "null".to_string(),
                |c| format!("[{},{}]", c.layout.width, c.layout.height)
            ),
            self.comp.as_ref().is_some_and(|c| c.final_res),
            self.comp.as_ref().map_or(0.0, |c| c.scale),
            opt(&self.layout.toggle_left),
            opt(&self.layout.toggle_right),
            opt(&self.layout.toggle_final),
            export_json,
            self.layout
                .tabs
                .iter()
                .map(rect)
                .collect::<Vec<_>>()
                .join(","),
            self.layout
                .boxes
                .iter()
                .map(|(id, r)| format!("{{\"id\":{},\"rect\":{}}}", id.0, rect(r)))
                .collect::<Vec<_>>()
                .join(","),
            self.zoom * ppp,
            self.left_open,
            self.right_open,
            self.status,
            ppp,
            ctx.input(|i| i.viewport().native_pixels_per_point)
                .unwrap_or(0.0),
            ctx.zoom_factor(),
        );
        if report != self.last_report {
            let _ = std::fs::write(path, &report);
            self.last_report = report;
        }
    }
}

// ---- composition preview and properties ---------------------------------------------------------

/// A quick preview is drawn much smaller than the final image; a final-size preview of an
/// enormous guide would need gigabytes, so it is limited.
const FINAL_PREVIEW_MAX_PIXELS: u64 = 40_000_000;

/// Below this window width (in points) the properties start folded away.
const NARROW_WINDOW: f32 = 900.0;

/// The frame of the side panels: the panel colour and a hairline, with `margin` inside.
fn panel_frame(ctx: &egui::Context, margin: i8) -> egui::Frame {
    egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::same(margin))
}

fn properties_open_by_default(window_width: f32) -> bool {
    window_width >= NARROW_WINDOW
}

/// The step whose box contains `p` (layout pixels).
fn box_at(layout: &ComposedLayout, p: guide_core::Point) -> Option<ItemId> {
    layout
        .boxes
        .iter()
        .find(|b| b.rect.contains(p))
        .map(|b| b.step)
}

/// Screen points per layout pixel that make the whole width fit; never enlarges past 100%.
fn fit_zoom(available_width: f32, layout_width: u32, pixels_per_point: f32) -> f32 {
    (available_width / layout_width.max(1) as f32).clamp(0.02, 1.0 / pixels_per_point.max(0.1))
}

fn combo<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    label: egui::Id,
    id: &str,
    current: &mut T,
    options: &[(T, &str)],
) -> bool {
    let before = *current;
    let shown = options
        .iter()
        .find(|(v, _)| v == current)
        .map_or("", |(_, l)| *l);
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (value, text) in options {
                ui.selectable_value(current, *value, *text);
            }
        })
        .response
        .labelled_by(label);
    *current != before
}

impl ProjectView {
    fn preview_state(&self) -> &'static str {
        if let Some((rev, final_res, _)) = &self.comp_error {
            if *rev == self.comp_rev && *final_res == self.final_preview {
                return "error";
            }
        }
        match &self.comp {
            None => "rendering",
            Some(c) if c.rev == self.comp_rev && c.final_res == self.final_preview => "ready",
            Some(_) => "stale",
        }
    }

    /// Collects finished previews and starts a new one when something changed.
    fn pump_composition(&mut self, ctx: &egui::Context) {
        if !self.composes() {
            return; // only a guide or a diff has a combined image
        }
        while let Ok(m) = self.comp_rx.try_recv() {
            self.comp_running = false;
            match m.result {
                Ok((layout, shot, scale)) => {
                    let tex = crate::app::texture_for(ctx, "guide-composition", &shot);
                    self.comp = Some(CompResult {
                        rev: m.rev,
                        final_res: m.final_res,
                        layout,
                        tex,
                        scale,
                    });
                    self.comp_error = None;
                }
                Err(e) => self.comp_error = Some((m.rev, m.final_res, e)),
            }
        }
        if !self.comp_running && matches!(self.preview_state(), "rendering" | "stale") {
            let age = self.comp_changed_at.elapsed();
            if age >= COMP_DEBOUNCE {
                self.start_composition(ctx);
            } else {
                ctx.request_repaint_after(COMP_DEBOUNCE - age);
            }
        }
        if self.comp_running {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }

    /// Draws the combined image on a background thread. The layout comes from the same
    /// engine the export uses; only the scale differs for the quick preview.
    fn start_composition(&mut self, ctx: &egui::Context) {
        self.comp_running = true;
        let (project, store, revs) = (self.project.clone(), self.store.clone(), self.revs.clone());
        let (cache, tx, ctx2) = (self.step_cache.clone(), self.comp_tx.clone(), ctx.clone());
        let (rev, final_res) = (self.comp_rev, self.final_preview);
        let missing_label = tr("Screenshot missing").to_string();
        thread::spawn(move || {
            let result = (|| -> Result<(ComposedLayout, Screenshot, f32), String> {
                let sizes = store.item_sizes(&project);
                let mut layout = if project.is_diff() {
                    compute_diff_layout(&project, &sizes)
                } else {
                    compute_layout(&project, &sizes)
                }
                .map_err(|e| e.to_string())?;
                layout.missing_label = missing_label;
                if final_res
                    && u64::from(layout.width) * u64::from(layout.height) > FINAL_PREVIEW_MAX_PIXELS
                {
                    return Err(tr_fmt(
                        "the final image would be {width}x{height} pixels; the final-size preview is limited to {megapixels} megapixels. The quick preview is still available.",
                        &[
                            ("width", &layout.width),
                            ("height", &layout.height),
                            ("megapixels", &(FINAL_PREVIEW_MAX_PIXELS / 1_000_000)),
                        ],
                    ));
                }
                let scale = if final_res {
                    1.0
                } else {
                    (PREVIEW_WIDTH / layout.width as f32).min(1.0)
                };
                let mut ask = |id: ItemId| -> Option<Image> {
                    let step = project.item(id)?;
                    let key = (id, revs.get(&id).copied().unwrap_or(0));
                    if !final_res {
                        if let Some(img) = cache.lock().ok()?.get(&key) {
                            return Some(Image::clone(img));
                        }
                    }
                    let o = store.load_asset(&step.image).ok()?;
                    let original = Image {
                        width: o.width,
                        height: o.height,
                        rgba: o.rgba,
                    };
                    let full = render(&original, &step.annotations).ok()?;
                    if final_res {
                        return Some(full);
                    }
                    let img = if full.width > CACHE_SIDE {
                        let h = (full.height as f32 * CACHE_SIDE as f32 / full.width as f32)
                            .round()
                            .max(1.0) as u32;
                        resample(&full, CACHE_SIDE, h)
                    } else {
                        full
                    };
                    let mut cache = cache.lock().ok()?;
                    if cache.len() > MAX_CACHED_PREVIEWS {
                        cache.clear();
                    }
                    cache.insert(key, Arc::new(img.clone()));
                    Some(img)
                };
                let image = render_layout(&layout, scale, &mut ask).map_err(|e| e.to_string())?;
                Ok((
                    layout,
                    Screenshot {
                        width: image.width,
                        height: image.height,
                        rgba: image.rgba,
                    },
                    scale,
                ))
            })();
            let _ = tx.send(CompMsg {
                rev,
                final_res,
                result,
            });
            ctx2.request_repaint();
        });
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let ppp = ctx.pixels_per_point();
        let is_guide = self.project.is_guide();
        let composed = self.composes();
        let pal = theme::palette(ui.ctx());
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(16, 8))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let h = 32.0;
                    let rail = if is_guide {
                        tr("Steps")
                    } else {
                        tr("Screenshots")
                    };
                    let r = theme::toggle_sized(ui, self.left_open, rail, egui::vec2(0.0, h))
                        .on_hover_text(if is_guide {
                            tr("Show or hide the step list")
                        } else {
                            tr("Show or hide the screenshot list")
                        });
                    if r.clicked() {
                        self.left_open = !self.left_open;
                    }
                    self.layout.toggle_left = Some(r.rect);
                    let r = theme::toggle_sized(
                        ui,
                        self.right_open,
                        tr("Properties"),
                        egui::vec2(0.0, h),
                    )
                    .on_hover_text(tr("Show or hide the properties"));
                    if r.clicked() {
                        self.right_open = !self.right_open;
                    }
                    self.layout.toggle_right = Some(r.rect);
                    if composed {
                        let r = theme::toggle_sized(
                            ui,
                            self.final_preview,
                            tr("Final-size preview"),
                            egui::vec2(0.0, h),
                        )
                        .on_hover_text(tr(
                            "Draw the exact image that will be exported, at full size (slower)",
                        ));
                        if r.clicked() {
                            self.final_preview = !self.final_preview;
                        }
                        self.layout.toggle_final = Some(r.rect);
                        if theme::Btn::new(tr("Fit"), theme::Emphasis::Secondary)
                            .show(ui)
                            .clicked()
                        {
                            self.auto_fit = true;
                        }
                        if theme::Btn::new("100%", theme::Emphasis::Secondary) // i18n-ok
                            .show(ui)
                            .clicked()
                        {
                            self.zoom = 1.0 / ppp;
                            self.auto_fit = false;
                        }
                        ui.label(
                            egui::RichText::new(format!("{:.0}%", self.zoom * ppp * 100.0))
                                .color(pal.muted),
                        );
                    }
                });
            });
        ui.separator();
        if !composed {
            if !self.status.is_empty() {
                ui.label(&self.status);
            }
            ui.separator();
            self.item_preview(ui, ctx);
            return;
        }
        ui.add_space(8.0);
        ui.horizontal_wrapped(
            |ui| match (self.preview_state(), &self.comp, &self.comp_error) {
                ("error", _, Some((_, _, e))) => {
                    ui.colored_label(
                        crate::theme::palette(ui.ctx()).error,
                        tr_fmt("Cannot draw the combined image: {error}", &[("error", &e)]),
                    );
                }
                (state, Some(c), _) => {
                    ui.label(tr_fmt(
                        "Output size: {width} x {height} px",
                        &[("width", &c.layout.width), ("height", &c.layout.height)],
                    ));
                    if state == "stale" || self.comp_running {
                        ui.label(tr("(updating...)"));
                    }
                    if c.final_res {
                        ui.label(tr("- exact final image"));
                    } else {
                        ui.label(tr_fmt(
                            "- quick preview at {percent}%",
                            &[("percent", &format!("{:.0}", c.scale * 100.0))],
                        ));
                    }
                }
                _ => {
                    ui.label(tr("Drawing the preview..."));
                }
            },
        );
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
        ui.add_space(4.0);

        let Some((tex_id, boxes, lw, lh)) = self.comp.as_ref().map(|c| {
            (
                c.tex.id(),
                c.layout.boxes.clone(),
                c.layout.width,
                c.layout.height,
            )
        }) else {
            ui.centered_and_justified(|ui| {
                if self.preview_state() == "error" {
                    ui.colored_label(
                        crate::theme::palette(ui.ctx()).error,
                        tr("The guide cannot be drawn with these settings."),
                    );
                } else {
                    ui.label(if self.project.is_diff() && self.project.items.is_empty() {
                        tr("Add the previous and the post screenshot to see the diff.")
                    } else if self.project.items.is_empty() {
                        tr("Add a step to see the guide: capture with Ctrl+N, then press \"Add to project\".")
                    } else {
                        tr("Drawing the preview...")
                    });
                }
            });
            return;
        };
        if self.auto_fit {
            self.zoom = fit_zoom(ui.available_width() - 14.0, lw, ppp);
        }
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if zoom_delta != 1.0 && ui.rect_contains_pointer(ui.available_rect_before_wrap()) {
            self.zoom = (self.zoom * zoom_delta).clamp(0.02, 8.0);
            self.auto_fit = false;
        }
        let zoom = self.zoom;
        let selected = self.selected;
        let mut clicked: Option<ItemId> = None;
        let mut screen_boxes = Vec::new();
        let mut picture = None;
        let scroll_target = (self.preview_scrolled_to != selected)
            .then_some(selected)
            .flatten();
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (rect, resp) = ui.allocate_exact_size(
                    Vec2::new(lw as f32 * zoom, lh as f32 * zoom),
                    Sense::click(),
                );
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Other,
                        true,
                        tr("Preview of the finished guide"),
                    )
                });
                picture = Some(rect);
                ui.painter().image(
                    tex_id,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(1.0, crate::theme::palette(ui.ctx()).border),
                    egui::StrokeKind::Outside,
                );
                for b in &boxes {
                    let r = Rect::from_min_size(
                        rect.min + Vec2::new(b.rect.x, b.rect.y) * zoom,
                        Vec2::new(b.rect.w, b.rect.h) * zoom,
                    );
                    screen_boxes.push((b.step, r));
                    if Some(b.step) == selected {
                        ui.painter().rect_stroke(
                            r.expand(3.0),
                            3.0,
                            egui::Stroke::new(2.0, crate::theme::palette(ui.ctx()).accent),
                            egui::StrokeKind::Outside,
                        );
                        if scroll_target == Some(b.step) {
                            ui.scroll_to_rect(r, Some(egui::Align::Center));
                        }
                    }
                }
                if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        let at = guide_core::Point::new(
                            (p.x - rect.min.x) / zoom,
                            (p.y - rect.min.y) / zoom,
                        );
                        clicked = screen_boxes
                            .iter()
                            .find(|(_, r)| r.contains(p))
                            .map(|(id, _)| *id)
                            .or_else(|| self.comp.as_ref().and_then(|c| box_at(&c.layout, at)));
                    }
                }
            });
        self.layout.boxes = screen_boxes;
        self.layout.preview = picture;
        self.preview_scrolled_to = selected;
        if let Some(id) = clicked {
            self.selected = Some(id);
            self.preview_scrolled_to = Some(id);
            self.scroll_to_selected = true; // bring it into view in the list too
        }
    }

    /// A plain project has no combined image: show the selected screenshot as it will be exported.
    fn item_preview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let ppp = ctx.pixels_per_point();
        let Some(id) = self.selected else {
            ui.centered_and_justified(|ui| {
                ui.label(if self.project.items.is_empty() {
                    tr("Add a screenshot: capture with Ctrl+N, then press \"Add to project\".")
                } else {
                    tr("Select a screenshot in the list.")
                });
            });
            return;
        };
        let Some((tex_id, size)) = self.previews.get(&id).map(|p| (p.tex.id(), p.size)) else {
            ui.centered_and_justified(|ui| {
                if let Some(issue) = self.issue_for(id) {
                    ui.colored_label(crate::theme::palette(ui.ctx()).error, &issue.message);
                } else {
                    ui.label(tr("Drawing the preview..."));
                }
            });
            return;
        };
        let fit = ((ui.available_width() - 14.0) * ppp / size.0.max(1) as f32).min(1.0);
        let shown = Vec2::new(size.0 as f32, size.1 as f32) * fit / ppp;
        let mut picture = None;
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (rect, resp) = ui.allocate_exact_size(shown, Sense::hover());
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Other,
                        true,
                        tr("Preview of the selected screenshot"),
                    )
                });
                ui.painter().image(
                    tex_id,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                picture = Some(rect);
            });
        self.layout.preview = picture;
    }

    fn properties(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        commands: &mut Vec<ProjectCommand>,
    ) {
        let is_guide = self.project.is_guide();
        if !is_guide
            && matches!(
                self.inspector_tab,
                InspectorTab::Layout | InspectorTab::Numbering
            )
        {
            self.inspector_tab = InspectorTab::Guide;
        }
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(16, 0))
            .show(ui, |ui| {
                let mut items = vec![
                    (
                        InspectorTab::Guide,
                        if is_guide { tr("Guide") } else { tr("Project") },
                    ),
                    (
                        InspectorTab::Step,
                        if is_guide {
                            tr("Step")
                        } else {
                            tr("Screenshot")
                        },
                    ),
                ];
                if is_guide {
                    items.push((InspectorTab::Layout, tr("Layout")));
                    items.push((InspectorTab::Numbering, tr("Numbering")));
                }
                self.layout.tabs = theme::tabs(ui, &mut self.inspector_tab, &items);
            });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(16, 8))
                    .show(ui, |ui| {
                        let mut changed = false;
                        match self.inspector_tab {
                            InspectorTab::Guide => changed |= self.guide_tab(ui),
                            InspectorTab::Step => changed |= self.step_tab(ui, commands),
                            InspectorTab::Layout => changed |= self.layout_tab(ui),
                            InspectorTab::Numbering => changed |= self.numbering_tab(ui),
                        }
                        // Last: used far less often than the layout controls, and the toolbar has an Export button.
                        ui.add_space(8.0);
                        self.export_section(ui, ctx);
                        self.issue_list(ui);
                        if changed {
                            self.mark_dirty();
                        }
                    });
            });
    }

    /// Title, introduction, final notes, importing, and what the export shows.
    fn guide_tab(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        let is_guide = self.project.is_guide();
        let untitled = self.untitled();
        let label = theme::section_label(ui, if is_guide { tr("Title") } else { tr("Name") });
        changed |= ui
            .add(
                crate::theme::text_singleline(&mut self.project.name)
                    .hint_text(untitled)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id)
            .changed();
        changed |= self.diff_settings(ui);
        if let Some(g) = self.project.guide_mut() {
            ui.add_space(6.0);
            let label = theme::section_label(ui, tr("Introduction"));
            changed |= ui
                .add(
                    crate::theme::text_multiline(&mut g.introduction)
                        .hint_text(tr("What will the reader learn?"))
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(label.id)
                .changed();
            ui.add_space(6.0);
            let label = theme::section_label(ui, tr("Final notes"));
            changed |= ui
                .add(
                    crate::theme::text_multiline(&mut g.final_notes)
                        .hint_text(tr("Wrap-up, links, next steps"))
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(label.id)
                .changed();
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(
                crate::theme::text_singleline(&mut self.import_path)
                    .font(egui::TextStyle::Monospace)
                    .hint_text(if is_guide {
                        tr("image file to add as a step")
                    } else {
                        tr("image file to add to the project")
                    })
                    .desired_width(150.0),
            )
            .widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::TextEdit,
                    true,
                    if is_guide {
                        tr("image file to add as a step")
                    } else {
                        tr("image file to add to the project")
                    },
                )
            });
            self.browsing.browse_button(
                ui,
                PathField::ImportImage,
                dialog::Kind::OpenImage,
                &self.import_path,
                if is_guide {
                    tr("Choose an image to add as a step")
                } else {
                    tr("Choose an image to add to the project")
                },
            );
            if ui
                .add_enabled(
                    !self.import_path.trim().is_empty(),
                    egui::Button::new(tr("Import")),
                )
                .clicked()
            {
                self.import();
            }
        });
        if is_guide {
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            theme::section_label(ui, tr("Show in export"));
            changed |= self.with_composition(ui, |ui, c, layout| {
                egui::Grid::new("show_in_export") // i18n-ok
                    .num_columns(2)
                    .spacing([8.0, 8.0])
                    .min_col_width((ui.available_width() - 8.0) / 2.0)
                    .show(ui, |ui| {
                        theme::checkbox_tile(ui, &mut c.show_numbers, tr("Step numbers"));
                        theme::checkbox_tile(ui, &mut c.show_titles, tr("Step titles"));
                        ui.end_row();
                        let r = theme::checkbox_tile(
                            ui,
                            &mut c.show_descriptions,
                            tr("Step descriptions"),
                        );
                        layout.show_descriptions = Some(r.rect);
                        theme::checkbox_tile(ui, &mut c.show_guide_title, tr("Guide title"));
                        ui.end_row();
                        theme::checkbox_tile(ui, &mut c.show_introduction, tr("Introduction"));
                        theme::checkbox_tile(ui, &mut c.show_final_notes, tr("Final notes"));
                        ui.end_row();
                    });
            });
        }
        changed
    }

    /// Header, arrangement and look of a diff. Does nothing for other kinds.
    fn diff_settings(&mut self, ui: &mut egui::Ui) -> bool {
        use crate::editor::props::{from_color32, to_color32};
        use guide_core::DiffLayout;
        let Some(d) = self.project.diff_mut() else {
            return false;
        };
        let before = d.clone();
        ui.add_space(6.0);
        theme::section_label(ui, tr("Arrangement"));
        ui.horizontal(|ui| {
            let label = ui.label(tr("Screenshots"));
            combo(
                ui,
                label.id,
                "diff-arrangement",
                &mut d.layout,
                &[
                    (DiffLayout::Horizontal, tr("side by side")),
                    (DiffLayout::Vertical, tr("one above the other")),
                ],
            );
        });
        ui.add_space(6.0);
        theme::checkbox_tile(ui, &mut d.show_header, tr("Header"));
        if d.show_header {
            let label = theme::section_label(ui, tr("Header title"));
            ui.add(
                crate::theme::text_singleline(&mut d.title)
                    .hint_text(tr("Defaults to the name"))
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
            let label = theme::section_label(ui, tr("Header description"));
            ui.add(
                crate::theme::text_multiline(&mut d.description)
                    .hint_text(tr("What changed?"))
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
        }
        ui.add_space(6.0);
        theme::checkbox_tile(ui, &mut d.show_captions, tr("Captions"));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(tr("Background"));
            let mut bg = to_color32(d.background);
            if ui.color_edit_button_srgba(&mut bg).changed() {
                d.background = from_color32(bg);
            }
            ui.label(tr("Text"));
            let mut fg = to_color32(d.text_color);
            if ui.color_edit_button_srgba(&mut fg).changed() {
                d.text_color = from_color32(fg);
            }
        });
        ui.horizontal(|ui| {
            let label = ui.label(tr("Margin"));
            ui.add(
                egui::DragValue::new(&mut d.margin)
                    .range(0..=300)
                    .suffix(" px"),
            )
            .labelled_by(label.id);
            let label = ui.label(tr("Spacing"));
            ui.add(
                egui::DragValue::new(&mut d.spacing)
                    .range(0..=300)
                    .suffix(" px"),
            )
            .labelled_by(label.id);
        });
        ui.horizontal(|ui| {
            let label = ui.label(tr("Text size"));
            ui.add(egui::DragValue::new(&mut d.font_size).range(8.0..=120.0))
                .labelled_by(label.id);
        });
        *d != before
    }

    /// The selected step: position, order buttons, text fields and per-step overrides.
    fn step_tab(&mut self, ui: &mut egui::Ui, commands: &mut Vec<ProjectCommand>) -> bool {
        let mut changed = false;
        let is_guide = self.project.is_guide();
        let selected = self
            .selected
            .and_then(|id| self.project.index_of(id).map(|i| (id, i)));
        let Some((id, index)) = selected else {
            ui.label(if is_guide {
                tr("Select a step in the list or in the preview.")
            } else {
                tr("Select a screenshot in the list.")
            });
            return false;
        };
        let total = self.project.items.len();
        let position = [
            ("number", &(index + 1) as &dyn std::fmt::Display),
            ("total", &total),
        ];
        ui.strong(if is_guide {
            tr_fmt("Step {number} of {total}", &position)
        } else {
            tr_fmt("Screenshot {number} of {total}", &position)
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(index > 0, egui::Button::new(tr("Move up")))
                .on_hover_text(tr("Alt+Up"))
                .clicked()
            {
                self.move_selected(-1);
            }
            if ui
                .add_enabled(index + 1 < total, egui::Button::new(tr("Move down")))
                .on_hover_text(tr("Alt+Down"))
                .clicked()
            {
                self.move_selected(1);
            }
            if ui
                .button(tr("Duplicate"))
                .on_hover_text(tr("Ctrl+D"))
                .clicked()
            {
                self.duplicate_selected();
            }
            if ui
                .button(tr("Remove"))
                .on_hover_text(tr("Delete key"))
                .clicked()
            {
                self.confirm_remove = Some(id);
            }
        });
        let broken = self.issue_for(id).is_some();
        if ui
            .add_enabled(!broken, egui::Button::new(tr("Edit screenshot (Enter)")))
            .clicked()
        {
            commands.push(ProjectCommand::EditStep(id));
        }
        let mut included = self.project.items[index].included;
        let r = theme::checkbox(ui, &mut included, tr("Include in export"));
        self.layout.included = Some(r.rect);
        if r.changed() {
            self.project.set_included(id, included);
            changed = true;
        }
        let step = &mut self.project.items[index];
        let label = theme::section_label(ui, tr("Title"));
        let title = ui
            .add(crate::theme::text_singleline(&mut step.title).desired_width(f32::INFINITY))
            .labelled_by(label.id);
        let label = theme::section_label(ui, tr("Description"));
        let description = ui
            .add(
                crate::theme::text_multiline(&mut step.description)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
        let label = theme::section_label(ui, tr("Caption"));
        let caption = ui
            .add(crate::theme::text_singleline(&mut step.caption).desired_width(f32::INFINITY))
            .labelled_by(label.id);
        changed |= title.changed() | description.changed() | caption.changed();
        self.layout.title = Some(title.rect);
        self.layout.description = Some(description.rect);
        if is_guide {
            ui.add_space(8.0);
            changed |= self.step_overrides(ui, id);
        }
        changed
    }

    /// Runs `add` on a copy of the guide's composition settings and stores the copy if it changed.
    fn with_composition(
        &mut self,
        ui: &mut egui::Ui,
        add: impl FnOnce(&mut egui::Ui, &mut guide_core::CompositionSettings, &mut Layout),
    ) -> bool {
        let Some(guide) = self.project.guide() else {
            return false;
        };
        let mut c = guide.composition.clone();
        add(ui, &mut c, &mut self.layout);
        match self.project.guide_mut() {
            Some(g) if c != g.composition => {
                g.composition = c;
                true
            }
            _ => false,
        }
    }

    /// Per-step overrides of the guide-wide settings. Returns whether anything changed.
    fn step_overrides(&mut self, ui: &mut egui::Ui, id: ItemId) -> bool {
        let Some(guide) = self.project.guide_mut() else {
            return false;
        };
        let current = guide.step_layouts.get(&id).cloned();
        let mut lay = current.clone().unwrap_or_default();
        let mut state = match lay.show_description {
            None => 0u8,
            Some(true) => 1,
            Some(false) => 2,
        };
        ui.horizontal(|ui| {
            let label = ui.label(tr("Description"));
            if combo(
                ui,
                label.id,
                "desc_override",
                &mut state,
                &[
                    (0, tr("as in the guide")),
                    (1, tr("always show")),
                    (2, tr("hide")),
                ],
            ) {
                lay.show_description = [None, Some(true), Some(false)][state as usize];
            }
        });
        let mut limit = lay.max_width.is_some();
        let mut width = lay.max_width.unwrap_or(600);
        ui.horizontal(|ui| {
            let checkbox = ui.checkbox(&mut limit, tr("Limit image width"));
            if checkbox.changed() {
                lay.max_width = limit.then_some(width);
            }
            if limit
                && ui
                    .add(egui::DragValue::new(&mut width).range(50..=10_000).speed(5))
                    .labelled_by(checkbox.id)
                    .changed()
            {
                lay.max_width = Some(width);
            }
        });
        let new_layout = (lay != StepLayout::default()).then_some(lay);
        if new_layout == current {
            return false;
        }
        match new_layout {
            Some(l) => guide.step_layouts.insert(id, l),
            None => guide.step_layouts.remove(&id),
        };
        true
    }

    /// The guide-wide arrangement, image placement, colours, margins and spacing.
    fn layout_tab(&mut self, ui: &mut egui::Ui) -> bool {
        self.with_composition(ui, |ui, c, layout| {
            theme::section_label(ui, tr("Layout and images"));
            let mut grid = matches!(c.layout, guide_core::Layout::Grid { .. });
            ui.horizontal(|ui| {
                let label = ui.label(tr("Arrangement"));
                if combo(
                    ui,
                    label.id,
                    "arrangement",
                    &mut grid,
                    &[(false, tr("One step per row")), (true, tr("Grid"))],
                ) {
                    c.layout = if grid {
                        guide_core::Layout::Grid { columns: 2 }
                    } else {
                        guide_core::Layout::Vertical
                    };
                }
            });
            if let guide_core::Layout::Grid { columns } = &mut c.layout {
                ui.horizontal(|ui| {
                    let label = ui.label(tr("Columns"));
                    ui.add(egui::DragValue::new(columns).range(1..=8))
                        .labelled_by(label.id);
                });
            }
            ui.horizontal(|ui| {
                let label = ui.label(tr("Content width"));
                let r = ui.add(
                    egui::DragValue::new(&mut c.max_width)
                        .range(200..=10_000)
                        .speed(5)
                        .suffix(" px"),
                );
                let r = r.labelled_by(label.id);
                layout.max_width = Some(r.rect);
            });
            ui.horizontal(|ui| {
                let label = ui.label(tr("Screenshots"));
                combo(
                    ui,
                    label.id,
                    "scaling",
                    &mut c.image_scaling,
                    &[
                        (ImageScaling::ShrinkToFit, tr("shrink to fit")),
                        (ImageScaling::FillWidth, tr("fill the width")),
                    ],
                );
            });
            ui.horizontal(|ui| {
                let label = ui.label(tr("Alignment"));
                combo(
                    ui,
                    label.id,
                    "align",
                    &mut c.image_align,
                    &[
                        (ImageAlign::Left, tr("left")),
                        (ImageAlign::Center, tr("centre")),
                        (ImageAlign::Right, tr("right")),
                    ],
                );
            });
            ui.horizontal(|ui| {
                let label = ui.label(tr("Description"));
                combo(
                    ui,
                    label.id,
                    "placement",
                    &mut c.description_placement,
                    &[
                        (DescriptionPlacement::AboveImage, tr("above the screenshot")),
                        (DescriptionPlacement::BelowImage, tr("below the screenshot")),
                    ],
                );
            });
            ui.add_space(8.0);
            theme::section_label(ui, tr("Background, spacing and margins"));
            use crate::editor::props::{from_color32, to_color32};
            ui.horizontal(|ui| {
                ui.label(tr("Background"));
                let mut bg = to_color32(c.background);
                if ui.color_edit_button_srgba(&mut bg).changed() {
                    c.background = from_color32(bg);
                }
                ui.label(tr("Text"));
                let mut fg = to_color32(c.text_color);
                if ui.color_edit_button_srgba(&mut fg).changed() {
                    c.text_color = from_color32(fg);
                }
            });
            ui.horizontal(|ui| {
                let label = ui.label(tr("Margin"));
                ui.add(
                    egui::DragValue::new(&mut c.margin)
                        .range(0..=300)
                        .suffix(" px"),
                )
                .labelled_by(label.id);
                let label = ui.label(tr("Spacing"));
                ui.add(
                    egui::DragValue::new(&mut c.spacing)
                        .range(0..=300)
                        .suffix(" px"),
                )
                .labelled_by(label.id);
            });
        })
    }

    /// Text size for the step numbers, titles and descriptions drawn on the guide.
    fn numbering_tab(&mut self, ui: &mut egui::Ui) -> bool {
        self.with_composition(ui, |ui, c, _| {
            theme::section_label(ui, tr("Numbering and text"));
            ui.horizontal(|ui| {
                // The slider's own text names both the slider and its number box.
                ui.add(egui::Slider::new(&mut c.font_size, 10.0..=48.0).text(tr("Font size")));
            });
        })
    }
}

// ---- export ----------------------------------------------------------------------------------------

impl ProjectView {
    fn export_options(&self) -> ExportOptions {
        let typed = self.export.dir.trim();
        ExportOptions {
            dir: if typed.is_empty() {
                self.store.dir().join("exports")
            } else {
                PathBuf::from(typed)
            },
            format: self.export.format,
            mode: self.export.mode,
            names: self.export.names,
            include_text: self.export.include_text,
            replace_existing: self.export.replace,
            missing_label: Some(tr("Screenshot missing").to_string()),
        }
    }

    /// Starts exporting on a background thread; the window keeps responding and shows progress.
    fn start_export(&mut self, ctx: &egui::Context) {
        if self.export.running.is_some() {
            return;
        }
        let options = self.export_options();
        if planned_files(&self.project, &options).is_empty() {
            // The toolbar button and Ctrl+E are not disabled like "Export now" is.
            self.status = tr("No steps are included in the export.").into();
            return;
        }
        let (project, store, ctx2) = (self.project.clone(), self.store.clone(), ctx.clone());
        let (tx, rx) = mpsc::channel();
        let cancel = Cancel::default();
        let thread_cancel = cancel.clone();
        thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress_ctx = ctx2.clone();
            let report =
                guide_export::export(&store, &project, &options, &thread_cancel, &mut |p| {
                    let _ = progress_tx.send(ExportMsg::Progress(p));
                    progress_ctx.request_repaint();
                });
            let _ = tx.send(ExportMsg::Done(report));
            ctx2.request_repaint();
        });
        self.export.running = Some(RunningExport { rx, cancel });
        self.export.runs += 1;
        self.export.last = None;
        self.export.progress = Some(Progress {
            done: 0,
            total: 1,
            message: tr("Starting").into(),
        });
    }

    fn pump_export(&mut self, ctx: &egui::Context) {
        let Some(run) = &self.export.running else {
            return;
        };
        let messages: Vec<ExportMsg> = run.rx.try_iter().collect();
        for m in messages {
            match m {
                ExportMsg::Progress(p) => self.export.progress = Some(p),
                ExportMsg::Done(report) => {
                    self.status = if report.cancelled {
                        tr_n(
                            "Export cancelled after {n} file",
                            "Export cancelled after {n} files",
                            report.written.len(),
                            &[],
                        )
                    } else if report.is_complete() {
                        tr_n(
                            "Exported {n} file",
                            "Exported {n} files",
                            report.written.len(),
                            &[],
                        )
                    } else {
                        tr_fmt(
                            "{files} with {problems}; see Export under Properties",
                            &[
                                (
                                    "files",
                                    &tr_n(
                                        "Exported {n} file",
                                        "Exported {n} files",
                                        report.written.len(),
                                        &[],
                                    ),
                                ),
                                (
                                    "problems",
                                    &tr_n(
                                        "{n} problem",
                                        "{n} problems",
                                        report.problems.len(),
                                        &[],
                                    ),
                                ),
                            ],
                        )
                    };
                    self.announce(&self.status);
                    self.export.last = Some(report);
                    self.export.running = None;
                    self.export.progress = None;
                    ctx.request_repaint();
                    return;
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn export_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let is_guide = self.project.is_guide();
        let composed = self.composes();
        if !composed {
            // Only a guide or a diff can be written as one combined image.
            self.export.mode = ExportMode::Separate;
        }
        theme::section_label(ui, tr("Export"));
        {
            ui.horizontal_wrapped(|ui| {
                let separate = if is_guide {
                    tr("One image per step")
                } else {
                    tr("One image per screenshot")
                };
                let a = theme::toggle_sized(
                    ui,
                    self.export.mode == ExportMode::Separate,
                    separate,
                    egui::vec2(0.0, 32.0),
                );
                if a.clicked() {
                    self.export.mode = ExportMode::Separate;
                }
                self.layout.export_separate = Some(a.rect);
                if composed {
                    let b = theme::toggle_sized(
                        ui,
                        self.export.mode == ExportMode::Combined,
                        tr("One combined image"),
                        egui::vec2(0.0, 32.0),
                    );
                    if b.clicked() {
                        self.export.mode = ExportMode::Combined;
                    }
                    self.layout.export_combined = Some(b.rect);
                }
            });
            ui.horizontal(|ui| {
                let label = ui.label(tr("Format"));
                combo(
                    ui,
                    label.id,
                    "export_format",
                    &mut self.export.format,
                    &[
                        (ImageFormat::Png, tr("PNG")),
                        (ImageFormat::Jpeg, tr("JPEG")),
                        (ImageFormat::WebP, tr("WebP (lossless)")),
                    ],
                );
            });
            let folder_label = ui.label(tr("Output folder"));
            ui.horizontal(|ui| {
                self.browsing.browse_button(
                    ui,
                    PathField::ExportFolder,
                    dialog::Kind::Folder,
                    &self.export.dir,
                    tr("Choose the output folder"),
                );
                ui.add(
                    crate::theme::text_singleline(&mut self.export.dir)
                        .hint_text(tr("the project's exports folder"))
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(folder_label.id);
            });
            if self.export.mode == ExportMode::Separate {
                let (with_title, number_only) = file_name_examples(self.export.format);
                ui.horizontal(|ui| {
                    let label = ui.label(tr("File names"));
                    combo(
                        ui,
                        label.id,
                        "export_names",
                        &mut self.export.names,
                        &[
                            (NameStyle::NumberAndTitle, with_title.as_str()),
                            (NameStyle::NumberOnly, number_only.as_str()),
                        ],
                    );
                });
                ui.checkbox(
                    &mut self.export.include_text,
                    tr("Draw the number, title and description on each image"),
                );
            }
            ui.checkbox(
                &mut self.export.replace,
                tr("Replace files with the same name"),
            )
            .on_hover_text(tr(
                "Off: an existing file is kept and the new one is written beside it as -2, -3...",
            ));

            let options = self.export_options();
            let plan = planned_files(&self.project, &options);
            if plan.is_empty() {
                ui.colored_label(
                    crate::theme::palette(ui.ctx()).error,
                    tr("No steps are included in the export."),
                );
            } else {
                ui.label(tr_n(
                    "Will write {n} file to {folder}:",
                    "Will write {n} files to {folder}:",
                    plan.len(),
                    &[("folder", &options.dir.display())],
                ));
                for name in plan.iter().take(6) {
                    ui.monospace(name);
                }
                if plan.len() > 6 {
                    ui.label(tr_fmt(
                        "... and {count} more",
                        &[("count", &(plan.len() - 6))],
                    ));
                }
                if self.export.mode == ExportMode::Combined {
                    if let Some(c) = &self.comp {
                        ui.label(tr_fmt(
                            "Image size {width} x {height} px",
                            &[("width", &c.layout.width), ("height", &c.layout.height)],
                        ));
                    }
                }
            }
            if self.export.running.is_none()
                && ui
                    .add_enabled(!plan.is_empty(), egui::Button::new(tr("Export now")))
                    .clicked()
            {
                self.start_export(ctx);
            }
            if let Some(report) = &self.export.last {
                ui.separator();
                let heading = if report.cancelled {
                    tr_n(
                        "Cancelled: {n} file written",
                        "Cancelled: {n} files written",
                        report.written.len(),
                        &[],
                    )
                } else if report.is_complete() {
                    tr_n(
                        "Done: {n} file written",
                        "Done: {n} files written",
                        report.written.len(),
                        &[],
                    )
                } else {
                    tr_fmt(
                        "{files}, {problems}",
                        &[
                            (
                                "files",
                                &tr_n(
                                    "{n} file written",
                                    "{n} files written",
                                    report.written.len(),
                                    &[],
                                ),
                            ),
                            (
                                "problems",
                                &tr_n("{n} problem", "{n} problems", report.problems.len(), &[]),
                            ),
                        ],
                    )
                };
                ui.strong(heading);
                for p in &report.problems {
                    ui.colored_label(crate::theme::palette(ui.ctx()).error, &p.message);
                }
                egui::CollapsingHeader::new(tr("Files written"))
                    .default_open(report.written.len() <= 6)
                    .show(ui, |ui| {
                        for w in report.written.iter().take(200) {
                            ui.label(format!("{} ({} x {})", w.path.display(), w.width, w.height));
                        }
                    });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws one frame of `v` with a toolbar that has nothing to capture.
    fn show_frame(v: &mut ProjectView, ctx: &egui::Context) -> Vec<ProjectCommand> {
        let (mut auto_add, mut options) = (false, false);
        v.show(
            ctx,
            &mut ProjectBar {
                auto_add: &mut auto_add,
                capture_options_open: &mut options,
                can_capture: true,
                capturing: false,
                countdown: None,
                app_status: "",
            },
        )
    }

    #[test]
    fn narrow_windows_start_with_the_properties_folded() {
        assert!(properties_open_by_default(1100.0));
        assert!(properties_open_by_default(900.0));
        assert!(!properties_open_by_default(899.0));
        assert!(!properties_open_by_default(640.0));
    }

    #[test]
    fn fit_zoom_fits_the_width_and_never_enlarges_past_100_percent() {
        assert!((fit_zoom(500.0, 1000, 1.0) - 0.5).abs() < 1e-6);
        assert_eq!(
            fit_zoom(5000.0, 1000, 1.0),
            1.0,
            "a small guide is shown at 100%, not stretched"
        );
        assert!(
            (fit_zoom(5000.0, 1000, 2.0) - 0.5).abs() < 1e-6,
            "100% is half a point per pixel on a 2x screen"
        );
        assert_eq!(
            fit_zoom(0.0, 1000, 1.0),
            0.02,
            "a collapsed area still gives a usable zoom"
        );
        assert!(
            fit_zoom(100.0, 0, 1.0) > 0.0,
            "an empty layout does not divide by zero"
        );
    }

    #[test]
    fn clicks_in_the_preview_find_the_step_under_them() {
        use guide_core::{Point, RectF};
        use guide_render::compose::StepBox;
        let layout = ComposedLayout {
            width: 100,
            height: 100,
            background: guide_core::Rgba([255; 4]),
            texts: vec![],
            badges: vec![],
            images: vec![],
            missing_label: String::new(),
            boxes: vec![
                StepBox {
                    step: ItemId(7),
                    number: 1,
                    rect: RectF {
                        x: 0.0,
                        y: 0.0,
                        w: 100.0,
                        h: 40.0,
                    },
                },
                StepBox {
                    step: ItemId(9),
                    number: 2,
                    rect: RectF {
                        x: 0.0,
                        y: 50.0,
                        w: 100.0,
                        h: 40.0,
                    },
                },
            ],
        };
        assert_eq!(box_at(&layout, Point::new(10.0, 10.0)), Some(ItemId(7)));
        assert_eq!(box_at(&layout, Point::new(99.0, 89.0)), Some(ItemId(9)));
        assert_eq!(
            box_at(&layout, Point::new(10.0, 45.0)),
            None,
            "the gap between steps selects nothing"
        );
        assert_eq!(box_at(&layout, Point::new(500.0, 500.0)), None);
    }

    #[test]
    fn any_guide_change_makes_the_combined_image_stale() {
        let (mut v, tmp) = view("stale");
        let before = v.comp_rev;
        v.add_capture(&shot(1));
        assert!(v.comp_rev > before, "adding a step must redraw the preview");
        let mid = v.comp_rev;
        v.project.guide_mut().unwrap().composition.margin += 4;
        v.mark_dirty();
        assert!(v.comp_rev > mid, "a layout setting must redraw the preview");
        let id = v.project.items[0].id;
        let mid = v.comp_rev;
        v.set_annotations(id, Document::default());
        assert_eq!(v.comp_rev, mid, "an unchanged annotation set does not");
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_click_marker_is_an_annotation_and_leaves_the_screenshot_alone() {
        let (mut v, tmp) = view("click-marker");
        v.add_capture_with_marker(&shot(2), Some((3, 4)), guide_core::Style::default());
        v.add_capture_with_marker(&shot(2), None, guide_core::Style::default());
        assert_eq!(v.project.items.len(), 2);
        let marked = v.project.items[0].id;
        let (original, doc) = v.load_for_editing(marked).unwrap();
        assert_eq!(original, shot(2), "the stored pixels are the capture");
        assert!(matches!(
            doc.annotations[0].kind,
            guide_core::Kind::Marker { center, .. } if center.x == 3.0 && center.y == 4.0
        ));
        let plain = v.project.items[1].id;
        assert!(v.load_for_editing(plain).unwrap().1.annotations.is_empty());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn slugs_are_safe_folder_names() {
        assert_eq!(slug("How to change a setting"), "how-to-change-a-setting");
        assert_eq!(slug("  --Weird!!  Title??  "), "weird-title");
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(slug("Ünïcode ñame"), "n-code-ame");
        assert_eq!(slug(""), "guide");
        assert_eq!(slug("!!!"), "guide");
        assert!(slug(&"a".repeat(500)).len() <= 60);
        assert!(slug("x/y\\z:*?")
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '-' || c.is_ascii_digit()));
    }

    #[test]
    fn new_project_dirs_never_collide() {
        let tmp = std::env::temp_dir().join(format!("snapforge-newdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let a = new_project_dir(&tmp, "My Guide");
        assert_eq!(
            a.file_name().unwrap().to_string_lossy(),
            format!("my-guide.{}", guide_core::PROJECT_EXTENSION)
        );
        std::fs::create_dir_all(&a).unwrap();
        let b = new_project_dir(&tmp, "My Guide");
        assert_eq!(
            b.file_name().unwrap().to_string_lossy(),
            format!("my-guide-2.{}", guide_core::PROJECT_EXTENSION)
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    fn shot(v: u8) -> Screenshot {
        Screenshot {
            width: 6,
            height: 4,
            rgba: [v, 0, 0, 255].repeat(24),
        }
    }

    fn view(tag: &str) -> (ProjectView, PathBuf) {
        let tmp =
            std::env::temp_dir().join(format!("snapforge-guideview-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        (
            ProjectView::create(&tmp, "Test guide", ProjectKind::Guide(Default::default()))
                .unwrap(),
            tmp,
        )
    }

    #[test]
    fn screen_readers_get_a_label_and_selected_state_for_every_step_row() {
        let (mut v, tmp) = view("a11y");
        v.add_capture(&shot(1));
        v.add_capture(&shot(2));
        v.project.items[0].title = "Open the menu".into();
        v.project.items[1].included = false;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut labels = Vec::new();
        for _ in 0..3 {
            let out = ctx.run(egui::RawInput::default(), |ctx| {
                show_frame(&mut v, ctx);
            });
            if let Some(update) = out.platform_output.accesskit_update {
                for (_, node) in update.nodes {
                    if let Some(l) = node.label() {
                        labels.push((l.to_string(), node.toggled()));
                    }
                }
            }
        }
        let find = |text: &str| labels.iter().find(|(l, _)| l.contains(text));
        assert!(find("Step 1: Open the menu").is_some(), "{labels:?}");
        let (second, selected) = find("Step 2: ").unwrap_or_else(|| panic!("{labels:?}"));
        assert!(second.contains("excluded"), "{second}");
        assert_eq!(
            *selected,
            Some(egui::accesskit::Toggled::True),
            "the new step is the selected one"
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn every_input_in_the_inspector_has_a_name_for_screen_readers() {
        use egui::accesskit::Role;
        let (mut v, tmp) = view("a11y-inspector");
        v.add_capture(&shot(1));
        v.right_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        ctx.set_pixels_per_point(1.0);
        let mut unnamed = Vec::new();
        let mut seen_roles = std::collections::BTreeSet::new();
        for tab in [
            InspectorTab::Guide,
            InspectorTab::Step,
            InspectorTab::Layout,
            InspectorTab::Numbering,
        ] {
            v.inspector_tab = tab;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 1200.0),
                )),
                ..Default::default()
            };
            let mut nodes = std::collections::HashMap::new();
            for _ in 0..3 {
                let out = ctx.run(input.clone(), |ctx| {
                    show_frame(&mut v, ctx);
                });
                if let Some(update) = out.platform_output.accesskit_update {
                    nodes.extend(update.nodes);
                }
            }
            for (id, node) in &nodes {
                let input_role = matches!(
                    node.role(),
                    Role::TextInput
                        | Role::MultilineTextInput
                        | Role::SpinButton
                        | Role::Slider
                        | Role::ComboBox
                );
                if !input_role {
                    continue;
                }
                seen_roles.insert(format!("{:?}", node.role()));
                let named = node.label().is_some_and(|l| !l.trim().is_empty())
                    || node.labelled_by().iter().any(|l| {
                        nodes
                            .get(l)
                            .and_then(|n| n.label().or_else(|| n.value()))
                            .is_some_and(|t| !t.trim().is_empty())
                    });
                if !named {
                    unnamed.push(format!("{tab:?}: {:?} {id:?}", node.role()));
                }
            }
        }
        assert!(
            seen_roles.len() >= 4,
            "the test must actually see inputs: {seen_roles:?}"
        );
        assert!(unnamed.is_empty(), "inputs without a name: {unnamed:#?}");
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// Runs two frames with a focused text field; `key` goes down in the second, which also
    /// calls `handle_keys`. Returns whether the field really had the keyboard.
    fn press_while_typing(v: &mut ProjectView, key: Key, ctrl: bool) -> bool {
        let ctx = egui::Context::default();
        let mut text = String::from("typing");
        let mut typing = false;
        for frame in 0..2 {
            let modifiers = if ctrl && frame == 1 {
                egui::Modifiers::COMMAND
            } else {
                egui::Modifiers::NONE
            };
            let events = if frame == 1 {
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }]
            } else {
                vec![]
            };
            let input = egui::RawInput {
                modifiers,
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let field = ui.text_edit_singleline(&mut text);
                    if frame == 0 {
                        field.request_focus();
                    }
                });
                if frame == 1 {
                    typing = ctx.wants_keyboard_input();
                    v.handle_keys(ctx, &mut Vec::new());
                }
            });
        }
        typing
    }

    #[test]
    fn ctrl_s_saves_while_a_text_field_has_the_keyboard_but_plain_keys_stay_with_the_field() {
        let (mut v, tmp) = view("typing");
        v.add_capture(&shot(1));
        assert!(v.is_dirty());
        assert!(
            press_while_typing(&mut v, Key::Delete, false),
            "the field has the keyboard"
        );
        assert!(v.confirm_remove.is_none(), "Delete belongs to the field");
        assert!(v.is_dirty());
        assert!(press_while_typing(&mut v, Key::S, true));
        assert!(
            !v.is_dirty(),
            "Ctrl+S saved although a field has the keyboard"
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn an_emptied_name_shows_a_stand_in_and_does_not_blank_the_window_title() {
        let (mut v, tmp) = view("untitled");
        assert_eq!(v.title(), "Test guide");
        v.project.name = "   ".into();
        assert_eq!(v.title(), tr("Untitled guide"));
        v.project.name.clear();
        assert_eq!(v.title(), tr("Untitled guide"));
        assert!(
            v.project.name.is_empty(),
            "the project itself is not renamed"
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn exporting_with_no_included_steps_starts_nothing_and_says_so() {
        let ctx = egui::Context::default();
        let (mut v, tmp) = view("export-empty");
        v.start_export(&ctx);
        assert!(v.export.running.is_none() && v.export.last.is_none());
        assert_eq!(v.status, tr("No steps are included in the export."));
        v.add_capture(&shot(1));
        let id = v.project.items[0].id;
        v.project.item_mut(id).unwrap().included = false;
        v.status.clear();
        v.start_export(&ctx);
        assert!(v.export.running.is_none(), "every step is excluded");
        assert!(!v.status.is_empty());
        v.project.item_mut(id).unwrap().included = true;
        v.start_export(&ctx);
        assert!(v.export.running.is_some(), "with a step it does start");
        v.stop_export(Duration::from_secs(10));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn the_file_name_examples_follow_the_chosen_image_format() {
        assert_eq!(
            file_name_examples(ImageFormat::Png),
            ("01-title.png".to_string(), "01.png".to_string())
        );
        assert_eq!(file_name_examples(ImageFormat::Jpeg).1, "01.jpg");
        assert_eq!(file_name_examples(ImageFormat::WebP).0, "01-title.webp");
    }

    /// Puts a fake export in `v` whose thread ends only when it is told to (or never).
    fn fake_running_export(v: &mut ProjectView, obeys_cancel: bool) -> mpsc::Sender<()> {
        let (tx, rx) = mpsc::channel();
        let cancel = Cancel::default();
        let (watch, done) = (cancel.clone(), tx.clone());
        let (release, released) = mpsc::channel::<()>();
        thread::spawn(move || {
            while !(obeys_cancel && watch.is_cancelled()) && released.try_recv().is_err() {
                thread::sleep(Duration::from_millis(5));
            }
            let _ = done.send(ExportMsg::Done(ExportReport::default()));
        });
        v.export.running = Some(RunningExport { rx, cancel });
        release
    }

    #[test]
    fn leaving_the_project_cancels_a_running_export_before_unused_images_are_deleted() {
        let (mut v, tmp) = view("close-export");
        v.add_capture(&shot(1));
        v.add_capture(&shot(2));
        let gone = v.project.items[0].id;
        let removed_image = v.project.item(gone).unwrap().image.clone();
        v.remove(gone);
        let _release = fake_running_export(&mut v, true);
        assert!(v.finish());
        assert!(v.export.running.is_none(), "the export was stopped");
        assert!(
            !v.dir().join(&removed_image).exists(),
            "and only then were unused images removed"
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn an_export_that_does_not_stop_in_time_keeps_the_unused_images() {
        let (mut v, tmp) = view("stuck-export");
        v.add_capture(&shot(1));
        v.add_capture(&shot(2));
        let gone = v.project.items[0].id;
        let removed_image = v.project.item(gone).unwrap().image.clone();
        v.remove(gone);
        let release = fake_running_export(&mut v, false);
        assert!(!v.stop_export(Duration::from_millis(50)));
        assert!(v.export.running.as_ref().unwrap().cancel.is_cancelled());
        assert!(v.dir().join(&removed_image).exists());
        release.send(()).unwrap();
        assert!(v.stop_export(Duration::from_secs(5)));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_failed_save_raises_a_banner_and_a_later_save_clears_it() {
        use std::os::unix::fs::PermissionsExt;
        let (mut v, tmp) = view("savefail");
        v.add_capture(&shot(1));
        let dir = v.dir().to_path_buf();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let failed = !v.save_now();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        if failed {
            assert!(v.save_error.is_some(), "the failure is kept for the banner");
            assert!(v.is_dirty(), "the changes are still waiting to be saved");
            assert!(v.save_now());
        }
        // (When the tests run as root the folder stays writable and there is nothing to show.)
        assert!(v.save_error.is_none());
        assert!(!v.is_dirty());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn captures_become_steps_after_the_selected_one() {
        let (mut v, tmp) = view("add");
        v.add_capture(&shot(1));
        v.add_capture(&shot(2));
        let (first, second) = (v.project.items[0].id, v.project.items[1].id);
        assert_eq!(v.selected, Some(second), "the new step is selected");
        v.select(Some(first));
        v.add_capture(&shot(3));
        let order: Vec<ItemId> = v.project.items.iter().map(|s| s.id).collect();
        assert_eq!(order[0], first);
        assert_eq!(
            order[2], second,
            "inserted right after the selected step, not at the end"
        );
        assert!(v.is_dirty());
        assert!(v.project.items.iter().all(|s| tmp
            .join(format!("test-guide.{}", guide_core::PROJECT_EXTENSION))
            .join(&s.image)
            .exists()));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn removing_selects_the_neighbour_and_keeps_images_until_the_guide_is_left() {
        let (mut v, tmp) = view("remove");
        for i in 0..3 {
            v.add_capture(&shot(i));
        }
        let ids: Vec<ItemId> = v.project.items.iter().map(|s| s.id).collect();
        let removed_image = v.project.item(ids[1]).unwrap().image.clone();
        v.remove(ids[1]);
        assert_eq!(v.selected, Some(ids[2]), "the step that took its place");
        v.remove(ids[2]);
        assert_eq!(
            v.selected,
            Some(ids[0]),
            "falls back to the previous one at the end"
        );
        let dir = v.dir().to_path_buf();
        assert!(
            dir.join(&removed_image).exists(),
            "the file stays while the guide is open"
        );
        assert!(v.finish());
        assert!(
            !dir.join(&removed_image).exists(),
            "unused images are cleaned up when leaving"
        );
        assert!(dir.join(&v.project.items[0].image).exists());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn duplicates_share_the_image_so_removing_one_keeps_the_file() {
        let (mut v, tmp) = view("dup");
        v.add_capture(&shot(9));
        let original = v.project.items[0].id;
        v.select(Some(original));
        v.duplicate_selected();
        let copy = v.selected.unwrap();
        assert_ne!(copy, original);
        let image = v.project.item(original).unwrap().image.clone();
        v.remove(original);
        assert!(v.finish());
        assert!(
            v.dir().join(&image).exists(),
            "the copy still needs the screenshot"
        );
        assert_eq!(v.project.item(copy).unwrap().image, image);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn selection_and_moves_stay_inside_the_list() {
        let (mut v, tmp) = view("nav");
        for i in 0..3 {
            v.add_capture(&shot(i));
        }
        v.select(Some(v.project.items[0].id));
        v.select_offset(-1);
        assert_eq!(v.selected, Some(v.project.items[0].id));
        v.select_offset(10);
        assert_eq!(v.selected, Some(v.project.items[2].id));
        let last = v.project.items[2].id;
        v.move_selected(5);
        assert_eq!(v.project.items[2].id, last, "cannot move past the end");
        v.move_selected(-1);
        assert_eq!(v.project.index_of(last), Some(1));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn saved_edits_survive_reopening_and_annotations_go_back_into_the_step() {
        let (mut v, tmp) = view("persist");
        v.add_capture(&shot(5));
        let id = v.project.items[0].id;
        v.project.item_mut(id).unwrap().title = "Click Save".into();
        let mut ed = guide_core::Editor::new(Document::default());
        ed.add(
            guide_core::Kind::Line {
                from: guide_core::Point::new(0.0, 0.0),
                to: guide_core::Point::new(3.0, 3.0),
            },
            guide_core::Style::default(),
        );
        v.set_annotations(id, ed.document().clone());
        let rev = v.rev(id);
        assert!(
            rev > 0,
            "the preview must be redrawn after an annotation change"
        );
        assert!(v.save_now() && !v.is_dirty());
        let dir = v.dir().to_path_buf();
        let reopened = ProjectView::open(&dir).unwrap();
        let s = reopened.project.item(id).unwrap();
        assert_eq!(
            (s.title.as_str(), s.annotations.annotations.len()),
            ("Click Save", 1)
        );
        let (shot_back, doc) = reopened.load_for_editing(id).unwrap();
        assert_eq!((shot_back.width, doc.annotations.len()), (6, 1));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn importing_a_missing_or_bad_file_reports_it_and_adds_nothing() {
        let (mut v, tmp) = view("import");
        v.import_path = "/nonexistent/file.png".into();
        v.import();
        assert!(v.status.starts_with("Could not import"), "{}", v.status);
        assert!(v.project.items.is_empty());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_plain_project_keeps_its_kind_and_never_draws_a_combined_image() {
        let tmp = std::env::temp_dir().join(format!("snapforge-plain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let mut v = ProjectView::create(&tmp, "", ProjectKind::Plain).unwrap();
        assert_eq!(v.title(), "Untitled project");
        v.add_capture(&shot(1));
        v.add_capture(&shot(2));
        let id = v.project.items[0].id;
        v.project.items[0].caption = "The first one".into();
        assert!(v.finish());

        let ctx = egui::Context::default();
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                show_frame(&mut v, ctx);
            });
        }
        assert!(
            v.comp.is_none() && !v.comp_running,
            "only a guide has a combined image"
        );
        v.export.mode = ExportMode::Combined;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            show_frame(&mut v, ctx);
        });
        assert_eq!(
            v.export.mode,
            ExportMode::Separate,
            "a plain project can only be exported as separate images"
        );

        let reopened = ProjectView::open(v.dir()).unwrap();
        assert!(!reopened.project.is_guide());
        assert_eq!(reopened.project.item(id).unwrap().caption, "The first one");
        assert_eq!(reopened.project.items.len(), 2);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
