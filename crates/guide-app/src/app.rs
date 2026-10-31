use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use crate::i18n::{tr, tr_fmt, tr_n};
use crate::{
    dialog,
    editor::{self, props::Props, EditorView},
    i18n,
    profile_ui::{ProfilePicker, ProfileRequest},
    project::{ProjectBar, ProjectCommand, ProjectView},
    theme,
};
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, ViewportCommand};
use guide_capture::{
    detect_backend, region_from_drag, run_capture, run_click_session, topmost_at, visible_rect,
    CancelToken, CaptureBackend, CaptureError, CaptureMode, CaptureRequest, Monitor, Rect,
    Screenshot, WindowInfo,
};
use guide_core::{
    canonical_key, find_duplicate, Action, ModeChoice, Shortcut, APP_NAME, APP_VERSION,
};
use guide_core::{Document, ProjectKind};
use guide_hotkeys::{HotkeyBackend, PointerBackend};
use guide_storage::{
    remember_recent, save_rgba, timestamped_stem, ClickCaptureOptions, EditBundle, EditorOptions,
    ImageFormat, Profile, ProfileError, Settings, ShortcutSettings, ThemeChoice, MAX_CLICK_SHOTS,
};

/// Settings are written this long after the last change, so typing in a
/// field does not rewrite the file on every keystroke.
const SAVE_DELAY: Duration = Duration::from_millis(500);

enum Event {
    Tick(u64),
    Done(Result<Screenshot, CaptureError>, Option<Vec<WindowInfo>>),
    /// Click capture: a screenshot and where the click was inside it.
    Shot(Screenshot, Option<(i32, i32)>),
    /// Click capture: the session is over after this many shots.
    SessionEnded(Result<usize, CaptureError>),
}

type SharedPointer = Arc<Mutex<Box<dyn PointerBackend>>>;

/// Frozen-frame region selection in progress.
struct Selection {
    /// `Some` = pick-a-window mode (hover and click) instead of dragging a region.
    windows: Option<Vec<WindowInfo>>,
    full: Screenshot,
    /// Top-left and size, in pixels, of the monitor shown inside `full`.
    origin: (u32, u32),
    size: (u32, u32),
    tex: TextureHandle,
    start: Option<egui::Pos2>,
    cur: Option<egui::Pos2>,
}

/// The shortcut actions are named by the core crate in English; the window shows them translated.
fn action_label(action: Action) -> &'static str {
    match action {
        Action::CaptureFullScreen => tr("Capture full screen"),
        Action::CaptureRegion => tr("Capture region"),
        Action::CaptureWindow => tr("Capture window"),
        Action::ToggleClickCapture => tr("Start or stop click capture"),
    }
}

fn format_label(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => tr("PNG"),
        ImageFormat::Jpeg => tr("JPEG"),
        ImageFormat::WebP => tr("WebP (lossless)"),
    }
}

/// The tabs of the home screen's main area.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HomeTab {
    Preview,
    Projects,
    SavedEdits,
}

/// Height of the full-width buttons under the capture mode grid.
const TOGGLE_HEIGHT_BTN: f32 = theme::TOGGLE_HEIGHT;

pub struct MainApp {
    backend: Arc<dyn CaptureBackend>,
    monitors: Vec<Monitor>,
    mode: ModeChoice,
    monitor_id: u32,
    delay_secs: u32,
    include_cursor: bool,
    format: ImageFormat,
    out_dir: String,
    rx: Option<mpsc::Receiver<Event>>,
    cancel: Option<CancelToken>,
    countdown: Option<u64>,
    capturing: bool,
    /// A click capture session is running (the window is minimized).
    click_session: bool,
    click_count: usize,
    /// Created on the first session; clicks are only watched while one runs.
    pointer: Option<SharedPointer>,
    /// The newest screenshot of the session, shown when it ends.
    click_last: Option<Screenshot>,
    click_marker: bool,
    click_max: u32,
    /// Named tool settings; the working values above and in the editor belong to `active_profile`.
    profiles: Vec<Profile>,
    active_profile: String,
    /// The editor's tool settings while no editor is open, and the last ones it reported.
    editor_options: EditorOptions,
    profile_picker: ProfilePicker,
    last: Option<(Screenshot, TextureHandle)>,
    show_help: bool,
    show_settings: bool,
    home_tab: HomeTab,
    /// The "Capture and files" window, shown while a project is open.
    capture_options_open: bool,
    /// `snapforge gui`: the shortcut list was turned off with F1.
    gui_help_hidden: bool,
    /// A folder dialog opened from a "Browse..." button.
    browsing: dialog::Pending<PathField>,
    /// `snapforge gui`: no main window; selection, then the editor, then quit.
    gui: bool,
    /// A failure the user must notice: shown in a dialog with the details to copy.
    error_dialog: Option<(String, String)>,
    /// Whether `last` has been saved, copied, annotated or added to a guide.
    last_kept: bool,
    /// The capture that `last` replaced while it was still unkept, so it can be brought back.
    previous: Option<(Screenshot, TextureHandle)>,
    /// Set while a region capture is running: pixel rect of the monitor to select on.
    pending_region: Option<Rect>,
    selection: Option<Selection>,
    status: String,
    editor: Option<EditorView>,
    open_path: String,
    project: Option<ProjectView>,
    /// Every finished capture also becomes a step of the open guide.
    auto_add: bool,
    new_project_title: String,
    /// The kind of project the "New project" form creates.
    new_project_is_guide: bool,
    project_path: String,
    shortcuts: ShortcutSettings,
    language: String,
    theme: ThemeChoice,
    recent_projects: Vec<String>,
    recent_edits: Vec<String>,
    hotkeys: Box<dyn HotkeyBackend>,
    hotkey_errors: HashMap<Action, String>,
    /// Action whose shortcut is being recorded.
    editing: Option<Action>,
    shortcut_msg: String,
    settings_path: Option<PathBuf>,
    saved: Settings,
    dirty_since: Option<Instant>,
}

impl MainApp {
    pub fn new(ctx: egui::Context) -> Self {
        Self::build(ctx, false)
    }

    /// `snapforge gui`: `shot` was captured before the window existed. Starts the region
    /// selection on it at once; the main window is never shown.
    pub fn new_gui(ctx: egui::Context, shot: Screenshot, options: crate::gui::GuiOptions) -> Self {
        let mut app = Self::build(ctx.clone(), true);
        if let Some(path) = options.path {
            app.out_dir = path.to_string_lossy().into_owned();
        }
        let monitor = crate::gui::overlay_monitor(&app.monitors, app.backend.pointer_position())
            .map(|m| Rect {
                x: m.x.max(0) as u32,
                y: m.y.max(0) as u32,
                w: m.width,
                h: m.height,
            });
        if let Some(m) = monitor.filter(|_| app.monitors.len() > 1) {
            // Put the overlay on the monitor holding the pointer (untested without a second monitor).
            let ppp = ctx.pixels_per_point();
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(
                m.x as f32 / ppp,
                m.y as f32 / ppp,
            )));
        }
        app.begin_selection(&ctx, shot, monitor, None);
        if app.selection.is_none() {
            // The capture could not be shown; there is nothing else to do in this mode.
            eprintln!("snapforge gui: {}", app.status);
            std::process::exit(1);
        }
        app
    }

    fn build(ctx: egui::Context, gui: bool) -> Self {
        let backend: Arc<dyn CaptureBackend> = Arc::from(detect_backend());
        let monitors = backend.monitors().unwrap_or_default();
        let caps = backend.capabilities();

        let settings_path = Settings::default_path().ok();
        let (mut settings, mut status) = match &settings_path {
            Some(p) => {
                let l = Settings::load(p);
                (l.settings, l.notice.unwrap_or_default())
            }
            None => (
                Settings::default(),
                tr("Settings cannot be saved: no home directory").to_string(),
            ),
        };
        // A saved mode may be unusable on this machine today (e.g. one monitor now).
        let usable = match settings.mode {
            ModeChoice::FullScreen => true,
            ModeChoice::Monitor => caps.monitor_select.is_yes() && monitors.len() > 1,
            ModeChoice::Region => caps.region.is_yes(),
            ModeChoice::Window => caps.window.is_yes(),
        };
        if !usable {
            settings.mode = ModeChoice::FullScreen;
        }
        if !caps.cursor.is_yes() {
            settings.include_cursor = false;
        }

        i18n::set_language(&i18n::detect(&settings.language));
        theme::install_fonts(&ctx);
        theme::apply(&ctx, settings.theme);
        let ctx_for_hotkeys = ctx;
        let hotkeys = guide_hotkeys::detect(move || ctx_for_hotkeys.request_repaint());
        let mut app = Self {
            backend,
            monitor_id: monitors.first().map_or(0, |m| m.id),
            monitors,
            mode: settings.mode,
            delay_secs: settings.delay_secs,
            include_cursor: settings.include_cursor,
            format: settings.format,
            out_dir: settings.out_dir.clone(),
            rx: None,
            cancel: None,
            countdown: None,
            capturing: false,
            click_session: false,
            click_count: 0,
            pointer: None,
            click_last: None,
            click_marker: settings.click_marker,
            click_max: settings.click_max,
            profiles: settings.profiles.clone(),
            active_profile: settings.active().name.clone(),
            editor_options: settings.active().editor_options.clone(),
            profile_picker: ProfilePicker::default(),
            last: None,
            show_help: false,
            show_settings: false,
            home_tab: HomeTab::Preview,
            capture_options_open: false,
            gui_help_hidden: false,
            browsing: dialog::Pending::default(),
            gui,
            error_dialog: None,
            last_kept: true,
            previous: None,
            pending_region: None,
            selection: None,
            status: String::new(),
            editor: None,
            open_path: String::new(),
            project: None,
            auto_add: false,
            new_project_title: String::new(),
            new_project_is_guide: true,
            project_path: String::new(),
            shortcuts: settings.shortcuts.clone(),
            language: settings.language.clone(),
            theme: settings.theme,
            recent_projects: settings.recent_projects.clone(),
            recent_edits: settings.recent_edits.clone(),
            hotkeys,
            hotkey_errors: HashMap::new(),
            editing: None,
            shortcut_msg: String::new(),
            settings_path,
            saved: settings,
            dirty_since: None,
        };
        if !gui {
            app.register_hotkeys();
        }
        if std::env::var_os("SNAPFORGE_TEST_OPEN_SETTINGS").is_some() {
            app.show_settings = true;
        }
        if let Some(dir) = std::env::var_os("SNAPFORGE_TEST_OPEN_GUIDE").filter(|_| !gui) {
            app.project_path = dir.to_string_lossy().into_owned();
            app.open_project();
            app.auto_add = std::env::var_os("SNAPFORGE_TEST_AUTO_ADD").is_some();
        }
        if status.is_empty() {
            if let Some((_, why)) = app.hotkey_errors.iter().next() {
                status = tr_fmt(
                    "Some global shortcuts are not active: {reason}",
                    &[("reason", &why)],
                );
            }
        }
        app.status = status;
        app
    }

    fn current_settings(&self) -> Settings {
        Settings {
            version: self.saved.version,
            out_dir: self.out_dir.clone(),
            format: self.format,
            delay_secs: self.delay_secs,
            include_cursor: self.include_cursor,
            mode: self.mode,
            shortcuts: self.shortcuts.clone(),
            language: self.language.clone(),
            theme: self.theme,
            recent_projects: self.recent_projects.clone(),
            recent_edits: self.recent_edits.clone(),
            click_marker: self.click_marker,
            click_max: self.click_max,
            profiles: self.profiles_with_working_values(),
            active_profile: Some(self.active_profile.clone()),
        }
    }

    /// The profiles, with the active one holding what is set in the window and the editor now.
    fn profiles_with_working_values(&self) -> Vec<Profile> {
        let mut profiles = self.profiles.clone();
        if let Some(p) = profiles.iter_mut().find(|p| p.name == self.active_profile) {
            p.click_capture_options = ClickCaptureOptions {
                marker: self.click_marker,
                max: self.click_max,
            };
            p.editor_options = self
                .editor
                .as_ref()
                .map_or_else(|| self.editor_options.clone(), |e| e.options());
        }
        profiles
    }

    /// Applies a profile change from the picker in the window or in the editor.
    fn change_profiles(&mut self, request: ProfileRequest) {
        let mut s = self.current_settings();
        let result = match request {
            ProfileRequest::Select(n) => s.select_profile(&n),
            ProfileRequest::New(n) => s.add_profile(&n),
            ProfileRequest::Rename { from, to } => s.rename_profile(&from, &to),
            ProfileRequest::Delete(n) => s.delete_profile(&n),
        };
        match result {
            Ok(()) => {
                s.sanitize(); // keeps the older top-level click fields in step
                let active = s.active();
                self.click_marker = active.click_capture_options.marker;
                self.click_max = active.click_capture_options.max;
                self.editor_options = active.editor_options.clone();
                self.active_profile = active.name.clone();
                self.profiles = s.profiles;
                if let Some(ed) = &mut self.editor {
                    ed.set_options(&self.editor_options);
                }
            }
            Err(e) => {
                self.status = match e {
                    ProfileError::EmptyName => tr("The profile name is empty"),
                    ProfileError::Duplicate => tr("A profile with that name already exists"),
                    ProfileError::Missing => tr("There is no profile with that name"),
                    ProfileError::LastProfile => tr("At least one profile must remain"),
                    ProfileError::TooMany => tr("There are too many profiles"),
                }
                .to_string();
            }
        }
    }

    /// Writes settings if they differ from the saved copy. Failures are shown
    /// in the status line and not retried until the next change.
    fn save_now(&mut self) {
        let current = self.current_settings();
        self.dirty_since = None;
        if current == self.saved {
            return;
        }
        if let Some(path) = &self.settings_path {
            if let Err(e) = current.save(path) {
                self.status = tr_fmt("Could not save settings: {error}", &[("error", &e)]);
            }
        }
        self.saved = current;
    }

    /// Saves shortly after the last change (see [`SAVE_DELAY`]).
    fn autosave(&mut self, ctx: &egui::Context) {
        if self.gui {
            return; // a one-shot capture must not rewrite the settings (e.g. with --path)
        }
        if self.current_settings() == self.saved {
            self.dirty_since = None;
            return;
        }
        let since = *self.dirty_since.get_or_insert_with(Instant::now);
        if since.elapsed() < SAVE_DELAY {
            ctx.request_repaint_after(SAVE_DELAY);
        } else {
            self.save_now();
        }
    }

    fn register_hotkeys(&mut self) {
        let bindings = self.shortcuts.bindings();
        self.hotkey_errors = self
            .hotkeys
            .register(&bindings)
            .into_iter()
            .filter_map(|(a, r)| r.err().map(|e| (a, e.to_string())))
            .collect();
    }

    fn on_hotkey(&mut self, ctx: &egui::Context, action: Action) {
        if action == Action::ToggleClickCapture {
            if self.click_session {
                if let Some(c) = &self.cancel {
                    c.cancel();
                }
            } else if !self.capturing && self.selection.is_none() {
                self.start_click_session(ctx);
            }
            return;
        }
        if self.capturing || self.click_session || self.selection.is_some() {
            return;
        }
        let caps = self.backend.capabilities();
        let (mode, support) = match action {
            Action::CaptureFullScreen => (ModeChoice::FullScreen, &caps.full_screen),
            Action::CaptureRegion => (ModeChoice::Region, &caps.region),
            Action::CaptureWindow => (ModeChoice::Window, &caps.window),
            Action::ToggleClickCapture => return,
        };
        match support {
            guide_capture::Support::Yes => self.start_capture(ctx, mode),
            guide_capture::Support::No(why) => {
                self.status = tr_fmt(
                    "{action}: {reason}",
                    &[("action", &action_label(action)), ("reason", &why)],
                );
            }
        }
    }

    fn begin_shortcut_edit(&mut self, action: Action) {
        // Release our grabs so the pressed keys reach this window instead of firing actions.
        self.hotkeys.register(&[]);
        self.editing = Some(action);
        self.shortcut_msg.clear();
    }

    fn finish_shortcut_edit(&mut self) {
        self.editing = None;
        self.register_hotkeys();
    }

    fn apply_shortcut(&mut self, action: Action, shortcut: Shortcut) {
        let mut bindings = self.shortcuts.bindings();
        bindings.retain(|(a, _)| *a != action);
        bindings.push((action, shortcut.clone()));
        if let Some((a, b)) = find_duplicate(&bindings) {
            let other = if a == action { b } else { a };
            self.shortcut_msg = tr_fmt(
                "{shortcut} is already used for \"{action}\"",
                &[("shortcut", &shortcut), ("action", &action_label(other))],
            );
            return;
        }
        self.shortcuts.set(action, &shortcut);
        self.shortcut_msg.clear();
        self.register_hotkeys();
    }

    /// Reads one key press while a shortcut is being recorded.
    fn recorded_shortcut(&mut self, ctx: &egui::Context) {
        let Some(action) = self.editing else { return };
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            let egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = ev
            else {
                continue;
            };
            if key == egui::Key::Escape {
                self.finish_shortcut_edit();
                return;
            }
            let Some(name) = canonical_key(key.name()) else {
                self.shortcut_msg = tr_fmt(
                    "The key {name} cannot be used in a shortcut",
                    &[("name", &key.name())],
                );
                return;
            };
            let mut text = String::new();
            for (on, m) in [
                (modifiers.ctrl, "Ctrl+"),
                (modifiers.alt, "Alt+"),
                (modifiers.shift, "Shift+"),
            ] {
                if on {
                    text.push_str(m);
                }
            }
            text.push_str(&name);
            match Shortcut::parse(&text) {
                Ok(sc) => {
                    self.finish_shortcut_edit();
                    self.apply_shortcut(action, sc);
                }
                Err(e) => self.shortcut_msg = e.to_string(),
            }
            return;
        }
    }

    /// Theme and language. Changes apply at once and are saved with the settings.
    fn appearance_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut changed = false;
        theme::card(ui, |ui| {
            theme::card_row_sized(ui, true, 52.0, |ui| {
                ui.label(tr("Theme"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::segmented(ui, |ui| {
                        // The enclosing right-to-left layout flips the order: list them backwards.
                        for (choice, label) in [
                            (ThemeChoice::Dark, tr("Dark")),
                            (ThemeChoice::Light, tr("Light")),
                            (ThemeChoice::System, tr("Follow the system")),
                        ] {
                            let r = theme::segment(
                                ui,
                                self.theme == choice,
                                label,
                                egui::vec2(0.0, 32.0),
                            );
                            if r.clicked() && self.theme != choice {
                                self.theme = choice;
                                changed = true;
                            }
                        }
                    });
                });
            });
            let languages = i18n::available();
            if languages.len() > 1 {
                theme::card_row_sized(ui, false, 52.0, |ui| {
                    ui.label(tr("Language"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let shown = if self.language.is_empty() {
                            tr("Automatic").to_string()
                        } else {
                            i18n::native_name(&self.language)
                        };
                        egui::ComboBox::from_id_salt("language")
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                let mut pick = |ui: &mut egui::Ui, code: &str, label: &str| {
                                    if ui.selectable_label(self.language == code, label).clicked() {
                                        self.language = code.to_string();
                                        i18n::set_language(&i18n::detect(code));
                                    }
                                };
                                pick(ui, "", tr("Automatic"));
                                for code in languages {
                                    pick(ui, code, &i18n::native_name(code));
                                }
                            });
                    });
                });
            }
        });
        if changed {
            theme::apply(&ctx, self.theme);
        }
    }

    /// Keeps the theme in step with the system theme.
    fn sync_appearance(&mut self, ctx: &egui::Context) {
        let want_dark = theme::use_dark(self.theme, ctx.system_theme());
        if want_dark != ctx.style().visuals.dark_mode {
            theme::apply(ctx, self.theme);
        }
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let pal = theme::palette(ui.ctx());
        if let Err(why) = self.hotkeys.availability() {
            ui.colored_label(
                pal.error,
                tr_fmt(
                    "Global shortcuts unavailable: {reason}",
                    &[("reason", &why)],
                ),
            );
            return;
        }
        ui.label(
            egui::RichText::new(tr_fmt(
                "Global: they work in any application while {app} is running. Click a shortcut to change it.",
                &[("app", &APP_NAME)],
            ))
            .color(pal.muted),
        );
        ui.add_space(4.0);
        theme::card(ui, |ui| {
            for (i, action) in Action::ALL.into_iter().enumerate() {
                theme::card_row_sized(ui, i == 0, 52.0, |ui| {
                    ui.label(action_label(action));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let reset = ui.add(
                            egui::Button::new(egui::RichText::new(tr("Reset")).color(pal.muted))
                                .frame(false),
                        );
                        if reset.clicked() {
                            if self.editing == Some(action) {
                                self.finish_shortcut_edit();
                            }
                            self.apply_shortcut(action, action.default_shortcut());
                        }
                        let editing = self.editing == Some(action);
                        let text = if editing {
                            tr("Press the new keys... (Esc cancels)").to_string()
                        } else {
                            self.shortcuts.get(action).to_string()
                        };
                        let key = egui::Button::new(egui::RichText::new(text).monospace())
                            .fill(pal.background)
                            .min_size(egui::vec2(0.0, 36.0));
                        if ui.add(key).clicked() && !editing {
                            self.begin_shortcut_edit(action);
                        }
                        if let Some(err) = self.hotkey_errors.get(&action) {
                            ui.colored_label(pal.error, err);
                        }
                    });
                });
            }
        });
        if !self.shortcut_msg.is_empty() {
            ui.colored_label(pal.error, &self.shortcut_msg);
        }
    }

    fn start_capture(&mut self, ctx: &egui::Context, mode: ModeChoice) {
        let request = CaptureRequest {
            include_cursor: self.include_cursor,
            mode: match mode {
                ModeChoice::FullScreen | ModeChoice::Region | ModeChoice::Window => {
                    CaptureMode::FullScreen
                }
                ModeChoice::Monitor => CaptureMode::Monitor(self.monitor_id),
            },
        };
        self.pending_region = matches!(mode, ModeChoice::Region | ModeChoice::Window)
            .then(|| self.monitor_under_window(ctx));
        let want_windows = mode == ModeChoice::Window;
        let (tx, rx) = mpsc::channel();
        let cancel = CancelToken::default();
        let (backend, token, ctx2) = (self.backend.clone(), cancel.clone(), ctx.clone());
        let delay = Duration::from_secs(self.delay_secs.into());
        // Hide our own window so it isn't part of the screenshot.
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        thread::spawn(move || {
            // Let the window manager finish minimizing before the countdown starts.
            thread::sleep(Duration::from_millis(400));
            let tick_tx = tx.clone();
            let tick_ctx = ctx2.clone();
            let mut result = run_capture(&*backend, &request, delay, &token, |s| {
                let _ = tick_tx.send(Event::Tick(s));
                tick_ctx.request_repaint();
            });
            // List windows right after the capture so the list matches the frozen image.
            let mut windows = None;
            if want_windows && result.is_ok() {
                match backend.windows() {
                    Ok(w) => windows = Some(w),
                    Err(e) => result = Err(e),
                }
            }
            let _ = tx.send(Event::Done(result, windows));
            ctx2.request_repaint();
        });
        self.rx = Some(rx);
        self.cancel = Some(cancel);
        self.capturing = true;
        self.status.clear();
    }

    /// Starts a session that takes a screenshot of every left click until it is
    /// stopped (shortcut, Esc, the Stop button) or `click_max` shots were taken.
    /// Region mode captures the full screen: there is nobody to drag a region.
    fn start_click_session(&mut self, ctx: &egui::Context) {
        let pointer = match &self.pointer {
            Some(p) => p.clone(),
            None => {
                let p = guide_hotkeys::detect_pointer(|| {});
                if let Err(why) = p.availability() {
                    self.status = tr_fmt(
                        "Click capture is not available: {reason}",
                        &[("reason", &why)],
                    );
                    return;
                }
                let p: SharedPointer = Arc::new(Mutex::new(p));
                self.pointer = Some(p.clone());
                p
            }
        };
        let request = CaptureRequest {
            include_cursor: self.include_cursor,
            mode: match self.mode {
                ModeChoice::FullScreen | ModeChoice::Region => CaptureMode::FullScreen,
                ModeChoice::Monitor => CaptureMode::Monitor(self.monitor_id),
                ModeChoice::Window => CaptureMode::Window(0),
            },
        };
        let max = (self.click_max > 0).then_some(self.click_max as usize);
        let (tx, rx) = mpsc::channel();
        let cancel = CancelToken::default();
        let (backend, token, ctx2) = (self.backend.clone(), cancel.clone(), ctx.clone());
        // Hide our own window so it isn't part of the screenshots.
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(400));
            // The click that started the session must not count.
            while pointer.lock().is_ok_and(|p| p.try_recv().is_some()) {}
            let (click_tx, click_rx) = mpsc::channel();
            let (forward_token, forward_from) = (token.clone(), pointer);
            thread::spawn(move || {
                while !forward_token.is_cancelled() {
                    let click = forward_from.lock().ok().and_then(|p| p.try_recv());
                    match click {
                        Some(c) if c.button == 1 => {
                            if click_tx.send((c.x, c.y)).is_err() {
                                break;
                            }
                        }
                        Some(_) => {}
                        None => thread::sleep(Duration::from_millis(10)),
                    }
                }
            });
            let (shot_tx, shot_ctx) = (tx.clone(), ctx2.clone());
            let result = run_click_session(
                &*backend,
                &request,
                &click_rx,
                &token,
                max,
                Duration::from_millis(150),
                |shot, at| {
                    let _ = shot_tx.send(Event::Shot(shot, at));
                    shot_ctx.request_repaint();
                },
            );
            token.cancel(); // also stops the click forwarder
            let _ = tx.send(Event::SessionEnded(result));
            ctx2.request_repaint();
        });
        self.rx = Some(rx);
        self.cancel = Some(cancel);
        self.click_session = true;
        self.click_count = 0;
        self.click_last = None;
        self.status.clear();
    }

    /// Files a click-capture screenshot as a step of the open guide, or in the output folder.
    fn take_click_shot(&mut self, shot: Screenshot, at: Option<(i32, i32)>) {
        self.click_count += 1;
        if let Some(g) = &mut self.project {
            let marker = guide_core::Style {
                fill: None,
                ..Props::from(&self.editor_options).style_for(false)
            };
            g.add_capture_with_marker(&shot, at.filter(|_| self.click_marker), marker);
        } else if let Err(e) = save_rgba(
            std::path::Path::new(&self.out_dir),
            &timestamped_stem(),
            self.format,
            shot.width,
            shot.height,
            &shot.rgba,
        ) {
            self.status = tr_fmt("Could not save the screenshot: {error}", &[("error", &e)]);
        }
        self.click_last = Some(shot);
    }

    fn end_click_session(&mut self, ctx: &egui::Context, result: Result<usize, CaptureError>) {
        self.rx = None;
        self.cancel = None;
        self.click_session = false;
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        if let Some(shot) = self.click_last.take() {
            let tex = texture_for(ctx, "shot", &shot);
            replace_capture(
                &mut self.last,
                &mut self.previous,
                &mut self.last_kept,
                (shot, tex),
                true,
            );
        }
        self.status = match result {
            Ok(_) => tr_n(
                "Click capture finished: {count} screenshot",
                "Click capture finished: {count} screenshots",
                self.click_count,
                &[("count", &self.click_count)],
            ),
            Err(e) => tr_fmt("Capture failed: {error}", &[("error", &e)]),
        };
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.rx else { return };
        let events: Vec<Event> = rx.try_iter().collect();
        for ev in events {
            match ev {
                Event::Shot(shot, at) => self.take_click_shot(shot, at),
                Event::SessionEnded(result) => {
                    self.end_click_session(ctx, result);
                    return;
                }
                Event::Tick(s) => self.countdown = Some(s),
                Event::Done(result, windows) => {
                    self.rx = None;
                    self.cancel = None;
                    self.capturing = false;
                    self.countdown = None;
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                    let region = self.pending_region.take();
                    match result {
                        Ok(shot) if region.is_some() => {
                            self.begin_selection(ctx, shot, region, windows)
                        }
                        Ok(shot) => self.show_capture(ctx, shot),
                        Err(CaptureError::Cancelled) => {
                            self.status = tr("Capture cancelled").into()
                        }
                        Err(e) => self.status = tr_fmt("Capture failed: {error}", &[("error", &e)]),
                    }
                    return;
                }
            }
        }
    }

    /// Pixel rect of the monitor the app window is on (falls back to the
    /// primary or first monitor, then to the whole desktop).
    fn monitor_under_window(&self, ctx: &egui::Context) -> Rect {
        let ppp = ctx.pixels_per_point();
        let center = ctx
            .input(|i| i.viewport().outer_rect)
            .map(|r| r.center() * ppp);
        let contains = |m: &&Monitor| {
            center.is_some_and(|c| {
                c.x >= m.x as f32
                    && c.y >= m.y as f32
                    && c.x < (m.x + m.width as i32) as f32
                    && c.y < (m.y + m.height as i32) as f32
            })
        };
        let m = self
            .monitors
            .iter()
            .find(contains)
            .or_else(|| self.monitors.iter().find(|m| m.primary))
            .or_else(|| self.monitors.first());
        match m {
            Some(m) => Rect {
                x: m.x.max(0) as u32,
                y: m.y.max(0) as u32,
                w: m.width,
                h: m.height,
            },
            None => Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            }, // resolved to the whole image later
        }
    }

    fn begin_selection(
        &mut self,
        ctx: &egui::Context,
        full: Screenshot,
        monitor: Option<Rect>,
        windows: Option<Vec<WindowInfo>>,
    ) {
        let monitor = monitor.filter(|r| r.w > 0 && r.h > 0).unwrap_or(Rect {
            x: 0,
            y: 0,
            w: full.width,
            h: full.height,
        });
        match full.crop(monitor) {
            Ok(view) => {
                self.selection = Some(Selection {
                    windows,
                    tex: texture_for(ctx, "selection", &view),
                    origin: (monitor.x, monitor.y),
                    size: (monitor.w, monitor.h),
                    full,
                    start: None,
                    cur: None,
                });
                ctx.send_viewport_cmd(ViewportCommand::Decorations(false));
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
                self.status.clear();
            }
            Err(e) => self.status = tr_fmt("Capture failed: {error}", &[("error", &e)]),
        }
    }

    fn end_selection(&mut self, ctx: &egui::Context) {
        self.selection = None;
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
        ctx.send_viewport_cmd(ViewportCommand::Decorations(true));
    }

    /// Draws the frozen frame and handles the drag; on mouse release the
    /// chosen region is cropped from the frozen capture.
    fn selection_ui(&mut self, ctx: &egui::Context) {
        let Some(sel) = self.selection.as_mut() else {
            return;
        };
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        if self.gui && ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            self.gui_help_hidden = !self.gui_help_hidden;
        }
        let show_help = self.gui && !self.gui_help_hidden && sel.start.is_none();
        let mut chosen = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Other,
                        true,
                        tr("Screen selection. Drag to choose the area to capture; press Escape to cancel."),
                    )
                });
                let painter = ui.painter();
                let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(sel.tex.id(), rect, full_uv, egui::Color32::WHITE);
                painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(120));
                let picking = sel.windows.is_some();
                if !picking && resp.drag_started() {
                    // egui reports the drag only after the pointer has moved a few
                    // points; the selection must start where the button went down.
                    sel.start = ctx
                        .input(|i| i.pointer.press_origin())
                        .or(resp.interact_pointer_pos());
                }
                if !picking && resp.dragged() {
                    sel.cur = resp.interact_pointer_pos();
                }
                let rel = |p: egui::Pos2| (p.x - rect.min.x, p.y - rect.min.y);
                let view = (rect.width(), rect.height());
                if let (Some(a), Some(b)) = (sel.start, sel.cur) {
                    let r = egui::Rect::from_two_pos(a, b).intersect(rect);
                    // Bright, undimmed pixels inside the selection.
                    let uv = egui::Rect::from_min_max(
                        egui::pos2(
                            (r.min.x - rect.min.x) / rect.width(),
                            (r.min.y - rect.min.y) / rect.height(),
                        ),
                        egui::pos2(
                            (r.max.x - rect.min.x) / rect.width(),
                            (r.max.y - rect.min.y) / rect.height(),
                        ),
                    );
                    painter.image(sel.tex.id(), r, uv, egui::Color32::WHITE);
                    painter.rect_stroke(
                        r,
                        0.0,
                        egui::Stroke::new(1.5, egui::Color32::LIGHT_BLUE),
                        egui::StrokeKind::Outside,
                    );
                    if let Some(px) = region_from_drag(rel(a), rel(b), view, sel.size, (0, 0)) {
                        painter.text(
                            r.min + egui::vec2(4.0, -4.0),
                            egui::Align2::LEFT_BOTTOM,
                            format!("{}x{}", px.w, px.h),
                            egui::FontId::proportional(14.0),
                            egui::Color32::WHITE,
                        );
                    }
                }
                if let (Some(windows), Some(p)) = (&sel.windows, resp.hover_pos()) {
                    let (fw, fh) = (sel.full.width, sel.full.height);
                    let (vx, vy) = rel(p);
                    let point = (
                        (sel.origin.0 as f32 + vx / view.0 * sel.size.0 as f32) as i32,
                        (sel.origin.1 as f32 + vy / view.1 * sel.size.1 as f32) as i32,
                    );
                    if let Some(w) = topmost_at(windows, point, (fw, fh)) {
                        if let Some(vis) = visible_rect(w.rect, (fw, fh)) {
                            let to_view = |x: u32, y: u32| {
                                egui::pos2(
                                    rect.min.x
                                        + (x as f32 - sel.origin.0 as f32) / sel.size.0 as f32
                                            * view.0,
                                    rect.min.y
                                        + (y as f32 - sel.origin.1 as f32) / sel.size.1 as f32
                                            * view.1,
                                )
                            };
                            let r = egui::Rect::from_min_max(
                                to_view(vis.x, vis.y),
                                to_view(vis.x + vis.w, vis.y + vis.h),
                            )
                            .intersect(rect);
                            let uv = egui::Rect::from_min_max(
                                egui::pos2(
                                    (r.min.x - rect.min.x) / view.0,
                                    (r.min.y - rect.min.y) / view.1,
                                ),
                                egui::pos2(
                                    (r.max.x - rect.min.x) / view.0,
                                    (r.max.y - rect.min.y) / view.1,
                                ),
                            );
                            painter.image(sel.tex.id(), r, uv, egui::Color32::WHITE);
                            painter.rect_stroke(
                                r,
                                0.0,
                                egui::Stroke::new(2.0, egui::Color32::LIGHT_BLUE),
                                egui::StrokeKind::Inside,
                            );
                            painter.text(
                                r.min + egui::vec2(6.0, 6.0),
                                egui::Align2::LEFT_TOP,
                                format!("{}  ({}x{})", w.title, vis.w, vis.h),
                                egui::FontId::proportional(14.0),
                                egui::Color32::WHITE,
                            );
                            if resp.clicked() {
                                chosen = Some(vis);
                            }
                        }
                    }
                }
                if show_help {
                    editor::show_gui_help(ctx, rect, true);
                }
                if sel.start.is_none() {
                    painter.text(
                        rect.center_top() + egui::vec2(0.0, 16.0),
                        egui::Align2::CENTER_TOP,
                        if picking {
                            tr("Click a window to capture it - Esc to cancel")
                        } else {
                            tr("Drag to select a region - Esc to cancel")
                        },
                        egui::FontId::proportional(16.0),
                        egui::Color32::WHITE,
                    );
                }
                if !picking && resp.drag_stopped() {
                    if let (Some(a), Some(b)) = (sel.start.take(), sel.cur.take()) {
                        chosen = region_from_drag(rel(a), rel(b), view, sel.size, sel.origin);
                    }
                }
            });
        if let Some(region) = chosen {
            let result = sel.full.crop(region);
            self.end_selection(ctx);
            match result {
                Ok(shot) if self.gui => self.open_gui_editor(ctx, &shot),
                Ok(shot) => self.show_capture(ctx, shot),
                Err(e) if self.gui => {
                    eprintln!("snapforge gui: capture failed: {e}");
                    std::process::exit(1);
                }
                Err(e) => self.status = tr_fmt("Capture failed: {error}", &[("error", &e)]),
            }
        } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.gui {
                // Close straight from fullscreen: leaving it first would show a small
                // decorated window for a moment, and so would a frame of the main window.
                ctx.send_viewport_cmd(ViewportCommand::Close);
            } else {
                self.end_selection(ctx);
                self.status = tr("Capture cancelled").into();
            }
        }
    }

    /// `snapforge gui`: the chosen region goes straight into the editor in a normal window.
    fn open_gui_editor(&mut self, ctx: &egui::Context, shot: &Screenshot) {
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(1100.0, 720.0)));
        let mut editor = EditorView::new(ctx, shot, Document::default(), None, timestamped_stem());
        editor.set_options(&self.editor_options);
        editor.refit_while_settling();
        editor.set_help_hidden(self.gui_help_hidden);
        self.editor = Some(editor);
    }

    /// Makes `shot` the current screenshot (preview, status line).
    fn show_capture(&mut self, ctx: &egui::Context, shot: Screenshot) {
        self.status = tr_fmt(
            "Captured {width}x{height}",
            &[("width", &shot.width), ("height", &shot.height)],
        );
        // Test hook for the end-to-end tests: write every finished capture to this folder.
        if let Some(dir) = std::env::var_os("SNAPFORGE_TEST_AUTOSAVE_DIR") {
            if let Err(e) = save_rgba(
                &PathBuf::from(dir),
                &timestamped_stem(),
                ImageFormat::Png,
                shot.width,
                shot.height,
                &shot.rgba,
            ) {
                self.status = tr_fmt("Test autosave failed: {error}", &[("error", &e)]);
            }
        }
        if self.auto_add {
            if let Some(g) = &mut self.project {
                g.add_capture(&shot);
            }
        }
        let tex = texture_for(ctx, "shot", &shot);
        self.home_tab = HomeTab::Preview;
        // A capture nobody saved is kept one step back instead of being lost silently.
        let kept = self.auto_add && self.project.is_some();
        replace_capture(
            &mut self.last,
            &mut self.previous,
            &mut self.last_kept,
            (shot, tex),
            kept,
        );
        if std::env::var_os("SNAPFORGE_TEST_OPEN_EDITOR").is_some() {
            self.open_editor(ctx);
        }
    }

    /// The product name, capture backend, shortcut help and settings, across the top.
    fn header_ui(&mut self, ui: &mut egui::Ui, caps: &guide_capture::Capabilities) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            theme::logo(ui);
            ui.label(
                egui::RichText::new(APP_NAME)
                    .size(20.0)
                    .family(theme::strong_family()),
            );
            let version = format!("v{APP_VERSION}"); // i18n-ok
            ui.label(egui::RichText::new(version).weak());
            let backend = format!("{} · {:.2}×", caps.backend, ui.ctx().pixels_per_point()); // i18n-ok
            theme::chip(ui, &backend).on_hover_text(tr_fmt(
                "Capture backend: {backend} - UI scale {scale}x",
                &[
                    ("backend", &caps.backend),
                    ("scale", &format!("{:.2}", ui.ctx().pixels_per_point())),
                ],
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::Btn::new("⚙", theme::Emphasis::Secondary) // i18n-ok
                    .name(tr("Settings"))
                    .show(ui)
                    .on_hover_text(tr("Settings"))
                    .clicked()
                {
                    self.show_settings = !self.show_settings;
                }
                if theme::Btn::new(tr("Shortcuts"), theme::Emphasis::Secondary)
                    .chip("F1") // i18n-ok
                    .show(ui)
                    .clicked()
                {
                    self.show_help = !self.show_help;
                }
            });
        });
    }

    /// Capture mode grid, the options card and the capture buttons: the left column of the home
    /// screen, and the "Capture and files" section while a project is open.
    fn capture_panel_ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        caps: &guide_capture::Capabilities,
        can_capture: bool,
    ) {
        let pal = theme::palette(ui.ctx());
        if let guide_capture::Support::No(why) = &caps.full_screen {
            ui.colored_label(
                pal.error,
                tr_fmt("Capture unavailable: {reason}", &[("reason", &why)]),
            );
        }
        theme::section_label(ui, tr("Capture mode"));
        let gap = ui.spacing().item_spacing.x;
        let w = (ui.available_width() - gap) / 2.0;
        let monitor_ok = caps.monitor_select.is_yes() && self.monitors.len() > 1;
        let modes = [
            (ModeChoice::FullScreen, tr("Full screen"), true, None),
            (ModeChoice::Region, tr("Region"), caps.region.is_yes(), None),
            (
                ModeChoice::Window,
                tr("Window"),
                caps.window.is_yes(),
                match &caps.window {
                    guide_capture::Support::No(why) => Some(why.clone()),
                    _ => None,
                },
            ),
            (ModeChoice::Monitor, tr("Single monitor"), monitor_ok, None),
        ];
        egui::Grid::new("capture-modes") // i18n-ok
            .spacing([gap, gap])
            .show(ui, |ui| {
                for (i, (mode, label, enabled, why)) in modes.into_iter().enumerate() {
                    let r = ui
                        .add_enabled_ui(enabled, |ui| {
                            theme::toggle(ui, self.mode == mode, label, w)
                        })
                        .inner;
                    if let Some(why) = why {
                        r.clone().on_disabled_hover_text(why);
                    }
                    if r.clicked() {
                        self.mode = mode;
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        if self.mode == ModeChoice::Monitor {
            egui::ComboBox::from_id_salt("monitor")
                .selected_text(
                    self.monitors
                        .iter()
                        .find(|m| m.id == self.monitor_id)
                        .map_or("?".into(), |m| m.name.clone()),
                )
                .show_ui(ui, |ui| {
                    for m in &self.monitors {
                        ui.selectable_value(&mut self.monitor_id, m.id, &m.name);
                    }
                });
        }
        ui.add_space(8.0);
        theme::card(ui, |ui| {
            theme::card_row(ui, true, |ui| {
                let cursor_ui = ui
                    .add_enabled_ui(caps.cursor.is_yes(), |ui| {
                        theme::checkbox(ui, &mut self.include_cursor, tr("Include cursor"))
                    })
                    .inner;
                if let guide_capture::Support::No(why) = &caps.cursor {
                    cursor_ui.on_disabled_hover_text(why);
                }
            });
            theme::card_row(ui, false, |ui| {
                theme::checkbox(ui, &mut self.click_marker, tr("Mark clicks"))
                    .on_hover_text(tr("Add a marker at the click to each new guide step"));
            });
            theme::card_row(ui, false, |ui| {
                let label = ui.label(tr("Delay"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(tr("sec")).color(pal.muted));
                    ui.add(egui::DragValue::new(&mut self.delay_secs).range(0..=60))
                        .labelled_by(label.id);
                });
            });
            theme::card_row(ui, false, |ui| {
                let label = ui.label(tr("Max shots"));
                ui.label(egui::RichText::new(tr("(0 = no limit)")).color(pal.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::DragValue::new(&mut self.click_max).range(0..=MAX_CLICK_SHOTS))
                        .labelled_by(label.id);
                });
            });
        });
        if !self.click_session {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let names: Vec<String> = self.profiles.iter().map(|p| p.name.clone()).collect();
                if let Some(r) =
                    self.profile_picker
                        .show(ui, "click-profile", &names, &self.active_profile)
                {
                    self.change_profiles(r);
                }
            });
        }
        // The capture buttons stay at the bottom of the column.
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            self.click_capture_ui(ui, ctx, can_capture);
            ui.add_space(4.0);
            ui.add_enabled_ui(can_capture, |ui| {
                if theme::Btn::new(tr("Capture"), theme::Emphasis::Accent)
                    .chip("Ctrl N") // i18n-ok
                    .name(tr("Capture (Ctrl+N)"))
                    .full_width(48.0)
                    .show(ui)
                    .clicked()
                {
                    self.start_capture(ctx, self.mode);
                }
            });
            if self.capturing || self.countdown.is_some() {
                ui.horizontal(|ui| {
                    if self.capturing && ui.button(tr("Cancel (Esc)")).clicked() {
                        if let Some(c) = &self.cancel {
                            c.cancel();
                        }
                    }
                    if let Some(s) = self.countdown {
                        ui.label(tr_fmt("Capturing in {seconds}...", &[("seconds", &s)]));
                    }
                });
            }
            if !self.status.is_empty() {
                ui.label(egui::RichText::new(&self.status).color(pal.muted));
            }
        });
    }

    /// Where the screenshot goes and what to do with it, pinned to the bottom of the home screen.
    fn save_bar_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = theme::palette(ui.ctx());
        let (hint, is_error) = match folder_state(&self.out_dir) {
            FolderState::Ready => ("", false),
            FolderState::WillBeCreated => (tr("(will be created)"), false),
            FolderState::NotAFolder => (tr("(not a folder)"), true),
            FolderState::NotWritable => (tr("(cannot write here)"), true),
            FolderState::Empty => (tr("(choose a folder)"), true),
        };
        if !hint.is_empty() {
            ui.colored_label(if is_error { pal.error } else { pal.muted }, hint);
        }
        let have = self.last.is_some();
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_add = have && self.project.is_some();
                ui.add_enabled_ui(can_add, |ui| {
                    if theme::Btn::new(tr("Add to project"), theme::Emphasis::Secondary)
                        .show(ui)
                        .on_hover_text(if self.project.is_some() {
                            tr("Add this screenshot to the open project")
                        } else {
                            tr("Open or create a project first")
                        })
                        .clicked()
                    {
                        self.add_last_to_project();
                    }
                });
                ui.add_enabled_ui(have, |ui| {
                    if theme::Btn::new(tr("Edit"), theme::Emphasis::Secondary)
                        .show(ui)
                        .on_hover_text(tr("Annotate this screenshot"))
                        .clicked()
                    {
                        self.open_editor(ctx);
                    }
                    if theme::Btn::new(tr("Copy"), theme::Emphasis::Secondary)
                        .show(ui)
                        .clicked()
                    {
                        self.copy();
                    }
                    if theme::Btn::new(tr("Save"), theme::Emphasis::Secondary)
                        .show(ui)
                        .clicked()
                    {
                        self.save();
                    }
                });
                ui.separator();
                egui::ComboBox::from_id_salt("format")
                    .selected_text(format_label(self.format))
                    .show_ui(ui, |ui| {
                        for f in ImageFormat::ALL {
                            ui.selectable_value(&mut self.format, f, format_label(f));
                        }
                    });
                self.browsing.browse_button(
                    ui,
                    PathField::OutDir,
                    dialog::Kind::Folder,
                    &self.out_dir,
                    tr("Choose the folder for screenshots"),
                );
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let label = ui.label(tr("Save to"));
                    ui.add(
                        crate::theme::text_singleline(&mut self.out_dir)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    )
                    .labelled_by(label.id);
                });
            });
        });
    }

    /// The "Projects" tab: the open project, or the form to create or open one.
    fn projects_tab_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(g) = &self.project {
            ui.label(tr_fmt(
                "Open project: {title} ({folder})",
                &[("title", &g.title()), ("folder", &g.dir().display())],
            ));
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.auto_add, tr("Add captures to the project"))
                    .on_hover_text(tr("Every screenshot you take is added to the project"));
                if ui.button(tr("Close project")).clicked() {
                    self.close_project();
                }
            });
            return;
        }
        ui.horizontal(|ui| {
            ui.label(tr("New project"));
            ui.selectable_value(&mut self.new_project_is_guide, true, tr("Guide"))
                .on_hover_text(tr(
                    "Numbered steps with an introduction, final notes and a combined image",
                ));
            ui.selectable_value(&mut self.new_project_is_guide, false, tr("Plain project"))
                .on_hover_text(tr("A set of screenshots to edit and export one by one"));
            ui.add(
                crate::theme::text_singleline(&mut self.new_project_title)
                    .hint_text(tr("title"))
                    .desired_width(220.0),
            );
            if ui.button(tr("Create")).clicked() {
                self.new_project();
            }
        });
        if let Some(path) = recent_list(ui, &mut self.recent_projects) {
            self.project_path = path;
            self.open_project();
        }
        ui.horizontal(|ui| {
            ui.label(tr("Open project"));
            ui.add(
                crate::theme::text_singleline(&mut self.project_path)
                    .font(egui::TextStyle::Monospace)
                    .hint_text(tr_fmt(
                        "path to a .{ext} folder",
                        &[("ext", &guide_core::PROJECT_EXTENSION)],
                    ))
                    .desired_width(320.0),
            );
            self.browsing.browse_button(
                ui,
                PathField::ProjectPath,
                dialog::Kind::Folder,
                &self.project_path,
                &tr_fmt(
                    "Choose a .{ext} folder",
                    &[("ext", &guide_core::PROJECT_EXTENSION)],
                ),
            );
            if ui
                .add_enabled(
                    !self.project_path.trim().is_empty(),
                    egui::Button::new(tr("Open")),
                )
                .clicked()
            {
                self.open_project();
            }
        });
    }

    /// The "Saved edits" tab: reopen an earlier annotated screenshot.
    fn saved_edits_tab_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(path) = recent_list(ui, &mut self.recent_edits) {
            self.open_path = path;
            self.open_saved_edit(ctx);
        }
        ui.horizontal(|ui| {
            ui.label(tr("Folder"));
            ui.add(
                crate::theme::text_singleline(&mut self.open_path)
                    .font(egui::TextStyle::Monospace)
                    .hint_text(tr_fmt(
                        "e.g. ~/Pictures/{app}/2026-01-01_12-00-00.{ext}",
                        &[("app", &APP_NAME), ("ext", &guide_core::EDIT_EXTENSION)],
                    ))
                    .desired_width(380.0),
            );
            self.browsing.browse_button(
                ui,
                PathField::OpenPath,
                dialog::Kind::Folder,
                &self.open_path,
                tr("Choose a saved edit folder"),
            );
            if ui
                .add_enabled(
                    !self.open_path.trim().is_empty(),
                    egui::Button::new(tr("Open")),
                )
                .clicked()
            {
                self.open_saved_edit(ctx);
            }
        });
    }

    /// Everything of the home screen in one block, for the "Capture and files" section while a
    /// project is open (the home screen itself lays the same parts out in panels).
    fn controls_ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        caps: &guide_capture::Capabilities,
        can_capture: bool,
    ) {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(304.0);
                self.capture_panel_ui(ui, ctx, caps, can_capture);
            });
            ui.vertical(|ui| {
                self.save_bar_ui(ui, ctx);
                self.saved_edits_tab_ui(ui, ctx);
            });
        });
    }

    /// Stop button while a click session runs, otherwise the secondary "Click capture" button.
    fn click_capture_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, can_capture: bool) {
        if self.click_session {
            if theme::Btn::new(tr("Stop click capture"), theme::Emphasis::Secondary)
                .full_width(TOGGLE_HEIGHT_BTN)
                .show(ui)
                .clicked()
            {
                if let Some(c) = &self.cancel {
                    c.cancel();
                }
            }
            ui.label(tr_n(
                "Click capture running: {count} screenshot",
                "Click capture running: {count} screenshots",
                self.click_count,
                &[("count", &self.click_count)],
            ));
            return;
        }
        ui.add_enabled_ui(can_capture, |ui| {
            if theme::Btn::new(tr("Click capture"), theme::Emphasis::Secondary)
                .note(tr("· one shot per click"))
                .full_width(TOGGLE_HEIGHT_BTN)
                .show(ui)
                .on_hover_text(tr_fmt(
                    "Take a screenshot at every left click. Stop with {shortcut}.",
                    &[("shortcut", &self.shortcuts.toggle_click_capture)],
                ))
                .clicked()
            {
                self.start_click_session(ctx);
            }
        });
    }

    /// The error dialog. It stays until closed, so a failure cannot scroll past unnoticed.
    fn error_window(&mut self, ctx: &egui::Context) {
        let Some((title, details)) = self.error_dialog.clone() else {
            return;
        };
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Window::new(title)
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(420.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                ui.label(&details);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tr("Copy details")).clicked() {
                        ui.ctx().copy_text(details.clone());
                    }
                    if ui.button(tr("Close")).clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.error_dialog = None;
        }
    }

    /// The settings page, opened by the gear button. It takes the place of the main view.
    fn settings_page(&mut self, ctx: &egui::Context) {
        let style = ctx.style();
        let header =
            egui::Frame::side_top_panel(&style).inner_margin(egui::Margin::symmetric(16, 10));
        egui::TopBottomPanel::top("controls") // i18n-ok
            .frame(header)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    if theme::Btn::new(&format!("‹  {}", tr("Back")), theme::Emphasis::Secondary) // i18n-ok
                        .name(tr("Back"))
                        .show(ui)
                        .clicked()
                    {
                        self.show_settings = false;
                    }
                    ui.label(
                        egui::RichText::new(tr("Settings"))
                            .size(20.0)
                            .family(theme::strong_family()),
                    );
                });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // A centred column of at most 720 points, and never wider than the window.
                    let width = ui.available_width().min(720.0);
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                theme::section_label(ui, tr("Shortcuts"));
                                self.settings_ui(ui);
                                ui.add_space(24.0);
                                theme::section_label(ui, tr("Appearance"));
                                ui.add_space(4.0);
                                self.appearance_ui(ui);
                                ui.add_space(24.0);
                            },
                        );
                    });
                });
        });
        // Esc cancels a pending shortcut edit first; only then does it leave the page.
        if self.editing.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_settings = false;
        }
    }

    /// The keyboard shortcut reference (F1). Every action here also has a button or menu.
    fn help_window(&mut self, ctx: &egui::Context) {
        if !self.show_help {
            return;
        }
        let mut open = true;
        let global = [
            (
                tr("Capture the screen"),
                self.shortcuts.capture_full_screen.clone(),
            ),
            (
                tr("Capture a region"),
                self.shortcuts.capture_region.clone(),
            ),
            (
                tr("Capture a window"),
                self.shortcuts.capture_window.clone(),
            ),
            (
                tr("Start or stop click capture"),
                self.shortcuts.toggle_click_capture.clone(),
            ),
        ];
        let anywhere = tr_fmt("Anywhere (while {app} is running)", &[("app", &APP_NAME)]);
        let sections: Vec<(&str, Vec<(&str, String)>)> = vec![
            (
                &anywhere,
                global.iter().map(|(a, k)| (*a, k.clone())).collect(),
            ),
            (
                tr("Main window"),
                vec![
                    (tr("Capture"), "Ctrl+N".into()),
                    (tr("Cancel a capture"), "Esc".into()),
                    (tr("Show or hide this help"), "F1".into()),
                    (tr("Move between controls"), "Tab / Shift+Tab".into()),
                    (tr("Press the focused control"), "Space / Enter".into()),
                ],
            ),
            (
                tr("Guide"),
                vec![
                    (tr("Previous or next step"), "Up / Down".into()),
                    (tr("Move the step up or down"), "Alt+Up / Alt+Down".into()),
                    (tr("Edit the step's screenshot"), "Enter".into()),
                    (tr("Duplicate the step"), "Ctrl+D".into()),
                    (tr("Remove the step"), "Delete".into()),
                    (tr("Save"), "Ctrl+S".into()),
                    (tr("Export"), "Ctrl+E".into()),
                ],
            ),
            (
                tr("Editor"),
                vec![
                    (tr("Choose a tool"), "V P H L A R E T M B C X".into()),
                    (tr("Undo or redo"), "Ctrl+Z / Ctrl+Shift+Z".into()),
                    (tr("Save"), "Ctrl+S".into()),
                    (tr("Export"), "Ctrl+E".into()),
                    (tr("Close the editor"), "Ctrl+W".into()),
                    (tr("Delete the selected annotation"), "Delete".into()),
                    (tr("Deselect, then back to the Select tool"), "Esc".into()),
                    (tr("Fit the image / actual size"), "0 / 1".into()),
                    (tr("Pan (hold and drag)"), "Space".into()),
                ],
            ),
        ];
        egui::Window::new(tr("Keyboard shortcuts"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(420.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (title, rows) in &sections {
                        ui.strong(*title);
                        egui::Grid::new(*title).num_columns(2).show(ui, |ui| {
                            for (what, keys) in rows {
                                ui.label(*what);
                                ui.monospace(keys);
                                ui.end_row();
                            }
                        });
                        ui.add_space(6.0);
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }
        self.show_help = open;
    }

    /// Shown before the first capture: what to do and which keys do it.
    fn empty_state_ui(&mut self, ui: &mut egui::Ui) {
        let pal = theme::palette(ui.ctx());
        ui.add_space((ui.available_height() / 4.0).max(8.0));
        ui.vertical_centered(|ui| {
            let card = egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(40, 32))
                .show(ui, |ui| {
                    ui.set_width(420.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(tr("Take your first screenshot"))
                                .size(24.0)
                                .family(theme::strong_family()),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(tr(
                                "Then annotate it, save it, or add it to a step-by-step guide.",
                            ))
                            .color(pal.muted),
                        );
                    });
                    ui.add_space(14.0);
                    for (label, shortcut) in [
                        (
                            tr("Full screen"),
                            self.shortcuts.capture_full_screen.to_string(),
                        ),
                        (tr("Region"), self.shortcuts.capture_region.to_string()),
                        (tr("Window"), self.shortcuts.capture_window.to_string()),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(label).color(pal.muted));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    theme::chip(ui, &shortcut);
                                },
                            );
                        });
                    }
                });
            let r = card.response.rect;
            let stroke = egui::Stroke::new(1.0, pal.border);
            let corners = [
                r.left_top(),
                r.right_top(),
                r.right_bottom(),
                r.left_bottom(),
                r.left_top(),
            ];
            ui.painter()
                .extend(egui::Shape::dashed_line(&corners, stroke, 6.0, 5.0));
        });
    }

    /// What the last capture is: its size and whether it has been kept.
    fn after_capture_ui(&mut self, ui: &mut egui::Ui) {
        let Some((shot, _)) = &self.last else { return };
        let size = tr_fmt(
            "Captured {width}x{height}",
            &[("width", &shot.width), ("height", &shot.height)],
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(size);
            if !self.last_kept {
                ui.colored_label(theme::palette(ui.ctx()).muted, tr("not saved yet"));
            }
        });
        if self.previous.is_some() {
            ui.horizontal(|ui| {
                ui.label(tr("The previous screenshot was not saved."));
                if ui.button(tr("Restore previous screenshot")).clicked() {
                    // Swap, so the newer capture can be brought back the same way.
                    restore_previous(&mut self.last, &mut self.previous, &mut self.last_kept);
                }
            });
        }
        ui.separator();
    }

    fn add_last_to_project(&mut self) {
        if let (Some((shot, _)), Some(g)) = (&self.last, &mut self.project) {
            g.add_capture(shot);
            self.last_kept = true;
        }
    }

    /// Puts the folder chosen in a "Browse..." dialog into the field it was opened for.
    fn take_browsed_path(&mut self) {
        if let Some((field, path)) = self.browsing.take_chosen() {
            let path = path.to_string_lossy().into_owned();
            match field {
                PathField::OutDir => self.out_dir = path,
                PathField::ProjectPath => self.project_path = path,
                PathField::OpenPath => self.open_path = path,
            }
        }
    }

    fn open_project(&mut self) {
        match ProjectView::open(&PathBuf::from(self.project_path.trim())) {
            Ok(g) => {
                remember_recent(&mut self.recent_projects, &g.dir().display().to_string());
                self.project = Some(g);
                self.status.clear();
            }
            Err(e) => {
                self.status = tr_fmt("Cannot open project: {error}", &[("error", &e)]);
                self.error_dialog = Some((tr("Cannot open the project").into(), e.to_string()));
            }
        }
    }

    fn new_project(&mut self) {
        match ProjectView::create(
            &PathBuf::from(&self.out_dir),
            &self.new_project_title,
            if self.new_project_is_guide {
                ProjectKind::Guide(Default::default())
            } else {
                ProjectKind::Plain
            },
        ) {
            Ok(g) => {
                self.status = tr_fmt("Created {folder}", &[("folder", &g.dir().display())]);
                remember_recent(&mut self.recent_projects, &g.dir().display().to_string());
                self.project = Some(g);
                self.new_project_title.clear();
            }
            Err(e) => {
                self.status = tr_fmt("Cannot create project: {error}", &[("error", &e)]);
                self.error_dialog = Some((tr("Cannot create the project").into(), e.to_string()));
            }
        }
    }

    /// Leaves the guide: saves it and deletes screenshots no step uses.
    fn close_project(&mut self) {
        if let Some(g) = &mut self.project {
            if g.finish() {
                self.project = None;
            } else {
                self.status = tr("The project could not be saved, so it stays open.").into();
            }
        }
    }

    fn edit_project_item(&mut self, ctx: &egui::Context, id: guide_core::ItemId) {
        let Some(g) = &mut self.project else { return };
        match g.load_for_editing(id) {
            Ok((shot, doc)) => self.editor = Some(EditorView::for_step(ctx, &shot, doc, id)),
            Err(e) => g.notify(tr_fmt(
                "Cannot edit this screenshot: {error}",
                &[("error", &e)],
            )),
        }
    }

    fn open_editor(&mut self, ctx: &egui::Context) {
        if let Some((shot, _)) = &self.last {
            self.last_kept = true;
            let mut editor =
                EditorView::new(ctx, shot, Document::default(), None, timestamped_stem());
            editor.set_options(&self.editor_options);
            self.editor = Some(editor);
        }
    }

    fn open_saved_edit(&mut self, ctx: &egui::Context) {
        match EditBundle::open(&PathBuf::from(self.open_path.trim())) {
            Ok((bundle, original, doc)) => {
                remember_recent(&mut self.recent_edits, &bundle.dir().display().to_string());
                let mut editor = EditorView::from_bundle(ctx, bundle, original, doc);
                editor.set_options(&self.editor_options);
                self.editor = Some(editor);
                self.status.clear();
            }
            Err(e) => {
                self.status = tr_fmt("Cannot open edit: {error}", &[("error", &e)]);
                self.error_dialog = Some((tr("Cannot open the saved edit").into(), e.to_string()));
            }
        }
    }

    fn save(&mut self) {
        let Some((shot, _)) = &self.last else { return };
        self.status = match save_rgba(
            &PathBuf::from(&self.out_dir),
            &timestamped_stem(),
            self.format,
            shot.width,
            shot.height,
            &shot.rgba,
        ) {
            Ok(p) => {
                self.last_kept = true;
                tr_fmt("Saved {path}", &[("path", &p.display())])
            }
            Err(e) => {
                self.error_dialog = Some((tr("Cannot save the screenshot").into(), e.to_string()));
                tr_fmt("Save failed: {error}", &[("error", &e)])
            }
        };
    }

    fn copy(&mut self) {
        let Some((shot, _)) = &self.last else { return };
        let img = guide_render::Image {
            width: shot.width,
            height: shot.height,
            rgba: shot.rgba.clone(),
        };
        self.status = match crate::clipboard::copy_image_persistent(&img) {
            Ok(()) => {
                self.last_kept = true;
                tr("Copied to the clipboard").into()
            }
            Err(e) => tr_fmt("Copy failed: {error}", &[("error", &e)]),
        };
    }
}

/// Makes `new` the current capture. The one it replaces is kept as `previous` unless it was
/// already saved, copied, annotated or added to a guide (`last_kept`), so nothing is lost silently.
fn replace_capture<T>(
    last: &mut Option<T>,
    previous: &mut Option<T>,
    last_kept: &mut bool,
    new: T,
    new_kept: bool,
) {
    let replaced = last.replace(new);
    *previous = if *last_kept { None } else { replaced };
    *last_kept = new_kept;
}

/// Swaps the current and previous captures; the one brought back counts as unkept again.
fn restore_previous<T>(last: &mut Option<T>, previous: &mut Option<T>, last_kept: &mut bool) {
    std::mem::swap(last, previous);
    *last_kept = false;
}

#[derive(Debug, PartialEq)]
enum FolderState {
    Ready,
    WillBeCreated,
    NotAFolder,
    NotWritable,
    Empty,
}

/// Whether screenshots can be saved into `path`, judged without creating anything.
fn folder_state(path: &str) -> FolderState {
    let path = path.trim();
    if path.is_empty() {
        return FolderState::Empty;
    }
    let p = std::path::Path::new(path);
    match std::fs::metadata(p) {
        Ok(m) if !m.is_dir() => FolderState::NotAFolder,
        Ok(m) if m.permissions().readonly() => FolderState::NotWritable,
        Ok(_) => FolderState::Ready,
        Err(_) => {
            // It will be created: the nearest folder that exists must be a writable folder.
            let parent = p.ancestors().skip(1).find(|a| a.exists());
            match parent.map(|a| (a, std::fs::metadata(a))) {
                Some((_, Ok(m))) if m.is_dir() && !m.permissions().readonly() => {
                    FolderState::WillBeCreated
                }
                Some((_, Ok(m))) if !m.is_dir() => FolderState::NotAFolder,
                _ => FolderState::NotWritable,
            }
        }
    }
}

/// The recently used folders as buttons; returns the one clicked. A folder that no longer exists
/// is shown greyed out with a button to forget it, instead of failing when clicked.
/// The text fields of the main window that have a "Browse..." button.
#[derive(Clone, Copy)]
enum PathField {
    OutDir,
    ProjectPath,
    OpenPath,
}

fn recent_list(ui: &mut egui::Ui, list: &mut Vec<String>) -> Option<String> {
    if list.is_empty() {
        return None;
    }
    let muted = theme::palette(ui.ctx()).muted;
    ui.label(tr("Recent"));
    let (mut open, mut forget) = (None, None);
    for (i, path) in list.iter().enumerate() {
        ui.horizontal(|ui| {
            if std::path::Path::new(path).exists() {
                if ui.link(path).on_hover_text(tr("Open")).clicked() {
                    open = Some(path.clone());
                }
            } else {
                ui.colored_label(muted, path);
                ui.label(tr("(missing)"));
                if ui.small_button(tr("Forget")).clicked() {
                    forget = Some(i);
                }
            }
        });
    }
    if let Some(i) = forget {
        list.remove(i);
    }
    open
}

impl eframe::App for MainApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Flush a change made within the last SAVE_DELAY.
        if !self.gui {
            self.save_now();
        }
        if let Some(g) = &mut self.project {
            g.finish();
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.sync_appearance(ctx);
        self.recorded_shortcut(ctx);
        while let Some(action) = self.hotkeys.try_recv() {
            self.on_hotkey(ctx, action);
        }
        self.autosave(ctx);
        self.take_browsed_path();
        if self.selection.is_some() {
            self.selection_ui(ctx);
            return;
        }
        // `snapforge gui` shows its own shortcut help in the editor and uses F1 for it.
        if !self.gui && ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            self.show_help = !self.show_help;
        }
        self.help_window(ctx);
        self.error_window(ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            if let Some(ed) = &mut self.editor {
                if ed.on_window_close_requested() {
                    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                }
            }
        }
        if let Some(ed) = &mut self.editor {
            let names: Vec<String> = self.profiles.iter().map(|p| p.name.clone()).collect();
            ed.set_profiles(&names, &self.active_profile);
            let env = editor::Env {
                out_dir: PathBuf::from(&self.out_dir),
                format: self.format,
                gui: self.gui,
            };
            let exit = ed.show(ctx, &env);
            self.editor_options = ed.options();
            let profile_request = ed.take_profile_request();
            if let Some(failure) = ed.take_failure() {
                self.error_dialog = Some((tr("Cannot finish the screenshot").into(), failure));
            }
            if let Some(dir) = ed.saved_dir() {
                let dir = dir.display().to_string();
                if self.recent_edits.first() != Some(&dir) {
                    remember_recent(&mut self.recent_edits, &dir);
                }
            }
            if let (Some((step, doc)), Some(g)) = (ed.take_commit(), &mut self.project) {
                g.set_annotations(step, doc);
            }
            if let editor::Exit::Close = exit {
                let quit = ed.quit_requested();
                self.editor = None;
                // Without another frame the guide's autosave timer would not start until the next input.
                ctx.request_repaint();
                if quit || self.gui {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
            if let Some(r) = profile_request {
                self.change_profiles(r);
            }
            return;
        }
        let caps = self.backend.capabilities();
        let can_capture = caps.full_screen.is_yes() && !self.capturing && !self.click_session;

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if let Some(c) = &self.cancel {
                c.cancel();
            }
        }
        if can_capture
            && !self.show_settings
            && self.editing.is_none()
            && ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::N))
        {
            self.start_capture(ctx, self.mode);
        }

        if self.show_settings {
            self.settings_page(ctx);
            return;
        }

        let style = ctx.style();
        if let Some(g) = &mut self.project {
            let mut bar = ProjectBar {
                auto_add: &mut self.auto_add,
                capture_options_open: &mut self.capture_options_open,
                can_capture,
                capturing: self.capturing,
                countdown: self.countdown,
                app_status: &self.status,
            };
            let commands = g.show(ctx, &mut bar);
            for c in commands {
                match c {
                    ProjectCommand::EditStep(id) => self.edit_project_item(ctx, id),
                    ProjectCommand::Capture => self.start_capture(ctx, self.mode),
                    ProjectCommand::CancelCapture => {
                        if let Some(c) = &self.cancel {
                            c.cancel();
                        }
                    }
                    ProjectCommand::Close => self.close_project(),
                    ProjectCommand::OpenSettings => self.show_settings = !self.show_settings,
                }
            }
            let mut open = self.capture_options_open && self.project.is_some();
            egui::Window::new(tr("Capture and files"))
                .open(&mut open)
                .default_width(640.0)
                .show(ctx, |ui| self.controls_ui(ui, ctx, &caps, can_capture));
            self.capture_options_open = open;
            return;
        }

        let header =
            egui::Frame::side_top_panel(&style).inner_margin(egui::Margin::symmetric(16, 10));
        egui::TopBottomPanel::top("header") // i18n-ok
            .frame(header)
            .show(ctx, |ui| self.header_ui(ui, &caps));

        let side = egui::Frame::side_top_panel(&style).inner_margin(egui::Margin::same(16));
        egui::SidePanel::left("capture") // i18n-ok
            .frame(side)
            .exact_width(304.0)
            .resizable(false)
            .show(ctx, |ui| self.capture_panel_ui(ui, ctx, &caps, can_capture));
        let bar = egui::Frame::side_top_panel(&style).inner_margin(egui::Margin::symmetric(16, 10));
        egui::TopBottomPanel::bottom("save-bar") // i18n-ok
            .frame(bar)
            .show(ctx, |ui| self.save_bar_ui(ui, ctx));
        let ppp = ctx.pixels_per_point();
        let central =
            egui::Frame::central_panel(&style).inner_margin(egui::Margin::symmetric(20, 0));
        egui::CentralPanel::default()
            .frame(central)
            .show(ctx, |ui| {
                theme::tabs(
                    ui,
                    &mut self.home_tab,
                    &[
                        (HomeTab::Preview, tr("Preview")),
                        (HomeTab::Projects, tr("Projects")),
                        (HomeTab::SavedEdits, tr("Saved edits")),
                    ],
                );
                ui.separator();
                ui.add_space(8.0);
                match self.home_tab {
                    HomeTab::Projects => self.projects_tab_ui(ui),
                    HomeTab::SavedEdits => self.saved_edits_tab_ui(ui, ctx),
                    HomeTab::Preview => {
                        if self.last.is_none() {
                            self.empty_state_ui(ui);
                            return;
                        }
                        self.after_capture_ui(ui);
                        if let Some((shot, tex)) = &self.last {
                            egui::ScrollArea::both().show(ui, |ui| {
                                // One image pixel = one screen pixel: sizes in points are pixels / ppp.
                                // The texture may be a shrunken copy, so size from the real capture.
                                let size = egui::vec2(shot.width as f32, shot.height as f32) / ppp;
                                let scale = (ui.available_width() / size.x).min(1.0);
                                ui.image((tex.id(), size * scale));
                            });
                        }
                    }
                }
            });
    }
}

/// Uploads `shot` as a texture. Images larger than the GPU's texture limit
/// (common for multi-monitor HiDPI desktops) are shrunk for display only;
/// egui panics on oversized textures, and the saved file keeps full size.
pub(crate) fn texture_for(ctx: &egui::Context, name: &str, shot: &Screenshot) -> TextureHandle {
    let max = ctx
        .input(|i| i.max_texture_side)
        .clamp(1, u32::MAX as usize) as u32;
    let scaled = shot.scaled_to_fit(max);
    let src = scaled.as_ref().unwrap_or(shot);
    let img =
        ColorImage::from_rgba_unmultiplied([src.width as usize, src.height as usize], &src.rgba);
    ctx.load_texture(name, img, TextureOptions::LINEAR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsaved_capture_survives_the_next_one_and_can_be_brought_back() {
        let (mut last, mut previous, mut kept) = (None, None, true);
        replace_capture(&mut last, &mut previous, &mut kept, "first", false);
        assert_eq!(
            (last, previous),
            (Some("first"), None),
            "nothing to keep yet"
        );
        replace_capture(&mut last, &mut previous, &mut kept, "second", false);
        assert_eq!(
            (last, previous),
            (Some("second"), Some("first")),
            "the unsaved first capture is kept"
        );
        replace_capture(&mut last, &mut previous, &mut kept, "third", false);
        assert_eq!(previous, Some("second"), "only one step back is kept");

        restore_previous(&mut last, &mut previous, &mut kept);
        assert_eq!(
            (last, previous, kept),
            (Some("second"), Some("third"), false)
        );
        restore_previous(&mut last, &mut previous, &mut kept);
        assert_eq!(last, Some("third"), "restoring twice returns to the newest");
    }

    #[test]
    fn a_saved_capture_is_not_kept_when_replaced() {
        let (mut last, mut previous, mut kept) = (Some("a"), None, true);
        replace_capture(&mut last, &mut previous, &mut kept, "b", false);
        assert_eq!((last, previous, kept), (Some("b"), None, false));
        let (mut last, mut previous, mut kept) = (None, None, true);
        replace_capture(&mut last, &mut previous, &mut kept, "auto", true);
        assert!(kept, "a capture added to a guide on arrival counts as kept");
    }

    #[test]
    fn the_save_folder_is_judged_without_creating_anything() {
        let tmp = std::env::temp_dir().join(format!("snapforge-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("a-file");
        std::fs::write(&file, b"x").unwrap();
        let s = |p: &std::path::Path| folder_state(&p.display().to_string());

        assert_eq!(folder_state("   "), FolderState::Empty);
        assert_eq!(s(&tmp), FolderState::Ready);
        assert_eq!(s(&tmp.join("new/deeper")), FolderState::WillBeCreated);
        assert!(!tmp.join("new").exists(), "checking must not create it");
        assert_eq!(s(&file), FolderState::NotAFolder);
        assert_eq!(s(&file.join("inside")), FolderState::NotAFolder);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
