//! Exporting a project to image files, with no UI.
//!
//! * **Separate images:** one file per included step, named `01-open-settings.png`.
//! * **Combined image:** the whole guide laid out as one picture by the composition engine
//!   (guides only).
//!
//! Files are written whole (temp file, then rename), nothing is deleted, and every step
//! that could not be exported is reported with the reason, so nothing is lost silently.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use guide_core::{
    slug, CompositionSettings, ImageScaling, Item, ItemId, Layout, Project, StepLayout,
};
use guide_render::{
    compose::{
        compute_layout, compute_layout_from, render_layout, render_layout_until, ComposeError,
    },
    render, Image,
};
use guide_storage::{write_rgba, ExistingFile, ImageFormat, ProjectStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportMode {
    /// One image per included step.
    Separate,
    /// The whole guide as one image.
    Combined,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NameStyle {
    /// `03-click-save.png`
    #[default]
    NumberAndTitle,
    /// `03.png`
    NumberOnly,
}

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub dir: PathBuf,
    pub format: ImageFormat,
    pub mode: ExportMode,
    pub names: NameStyle,
    /// Separate mode only: draw the step number, title and description around each screenshot.
    pub include_text: bool,
    /// `false` (the safe default) keeps existing files and writes `-2`, `-3`... beside them.
    pub replace_existing: bool,
    /// Combined mode: the words on the placeholder of a missing screenshot. `None` keeps the
    /// English default; the app passes its translation.
    pub missing_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub message: String,
}

/// A flag that asks a running export to stop at the next safe point.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub step: Option<ItemId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub step: Option<ItemId>,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportReport {
    pub written: Vec<Written>,
    pub problems: Vec<Problem>,
    pub cancelled: bool,
}

impl ExportReport {
    /// Everything that was asked for was written and nothing went wrong.
    pub fn is_complete(&self) -> bool {
        !self.cancelled && self.problems.is_empty() && !self.written.is_empty()
    }

    fn problem(&mut self, step: Option<ItemId>, message: impl Into<String>) {
        self.problems.push(Problem {
            step,
            message: message.into(),
        });
    }
}

/// The file name (without extension) for step number `number` of `count`. Numbers are padded
/// to at least two digits, and wider for 100 or more steps, so names sort in guide order.
pub fn file_stem(number: usize, count: usize, title: &str, style: NameStyle) -> String {
    let width = count.to_string().len().max(2);
    match style {
        NameStyle::NumberOnly => format!("{number:0width$}"),
        NameStyle::NumberAndTitle => {
            let title = slug(title, 50, "");
            if title.is_empty() {
                format!("{number:0width$}-step")
            } else {
                format!("{number:0width$}-{title}")
            }
        }
    }
}

/// The file names an export would produce, in order, before any `-2` suffix for a clash with
/// an existing file. Lets the window show them before the user commits.
pub fn planned_files(project: &Project, options: &ExportOptions) -> Vec<String> {
    let ext = options.format.extension();
    match options.mode {
        ExportMode::Combined => vec![format!("{}.{ext}", combined_stem(project))],
        ExportMode::Separate => {
            let steps: Vec<&Item> = project.included_items().collect();
            steps
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    format!(
                        "{}.{ext}",
                        file_stem(i + 1, steps.len(), &s.title, options.names)
                    )
                })
                .collect()
        }
    }
}

/// The file name (without extension) for the combined image.
pub fn combined_stem(project: &Project) -> String {
    slug(&project.name, 60, "guide")
}

fn label(step: &Item, number: usize) -> String {
    if step.title.trim().is_empty() {
        format!("Step {number}")
    } else {
        format!("Step {number} \"{}\"", step.title.trim())
    }
}

/// A step's finished screenshot: original with its annotations drawn and its crop applied.
fn finished_image(store: &ProjectStore, step: &Item) -> Result<Image, String> {
    let o = store.load_asset(&step.image).map_err(|e| e.to_string())?;
    let original = Image {
        width: o.width,
        height: o.height,
        rgba: o.rgba,
    };
    render(&original, &step.annotations).map_err(|e| e.to_string())
}

/// Draws the step number, title and description around a finished screenshot, with the
/// guide's own composition settings (minus the guide-level title and notes). A plain project
/// uses the default settings and no number.
fn with_text(project: &Project, step: &Item, number: usize, image: Image) -> Result<Image, String> {
    let mut single = Project::new_guide("");
    let mut settings = project
        .guide()
        .map_or_else(CompositionSettings::default, |g| g.composition.clone());
    settings.show_guide_title = false;
    settings.show_introduction = false;
    settings.show_final_notes = false;
    settings.layout = Layout::Vertical;
    settings.image_scaling = ImageScaling::ShrinkToFit;
    settings.show_numbers = settings.show_numbers && project.is_guide();
    // Wide enough for the screenshot at its natural size, and for text to wrap sensibly.
    settings.max_width = image.width.max(480);
    let mut s = step.clone();
    s.included = true;
    let layout = StepLayout {
        max_width: None,
        show_description: project
            .step_layout(step.id)
            .and_then(|l| l.show_description),
    };
    let sizes = [(s.id, (image.width, image.height))].into_iter().collect();
    let guide = single.guide_mut().expect("a new guide has guide settings");
    guide.composition = settings;
    guide.step_layouts.insert(s.id, layout);
    single.items.push(s);
    let layout = compute_layout_from(&single, &sizes, number as u32).map_err(|e| e.to_string())?;
    let mut slot = Some(image);
    render_layout(&layout, 1.0, &mut |_| slot.take()).map_err(|e| e.to_string())
}

/// Exports `project` as asked. Never panics on bad input and never deletes anything; check
/// the returned report for what was written, what failed and whether it was cancelled.
pub fn export(
    store: &ProjectStore,
    project: &Project,
    options: &ExportOptions,
    cancel: &Cancel,
    progress: &mut dyn FnMut(Progress),
) -> ExportReport {
    let mut report = ExportReport::default();
    let steps: Vec<&Item> = project.included_items().collect();
    if steps.is_empty() {
        report.problem(
            None,
            "No steps are included in the export. Tick \"Include in export\" on at least one step.",
        );
        return report;
    }
    if options.mode == ExportMode::Combined && !project.is_guide() {
        report.problem(
            None,
            "Only a guide can be exported as one combined image. Export the screenshots as separate images instead.",
        );
        return report;
    }
    if let Err(e) = std::fs::create_dir_all(&options.dir) {
        report.problem(
            None,
            format!(
                "Cannot use the output folder {}: {e}",
                options.dir.display()
            ),
        );
        return report;
    }
    let existing = if options.replace_existing {
        ExistingFile::Replace
    } else {
        ExistingFile::KeepBoth
    };
    match options.mode {
        ExportMode::Separate => separate(
            store,
            project,
            &steps,
            options,
            existing,
            cancel,
            progress,
            &mut report,
        ),
        ExportMode::Combined => combined(
            store,
            project,
            &steps,
            options,
            existing,
            cancel,
            progress,
            &mut report,
        ),
    }
    report
}

#[allow(clippy::too_many_arguments)]
fn separate(
    store: &ProjectStore,
    project: &Project,
    steps: &[&Item],
    options: &ExportOptions,
    existing: ExistingFile,
    cancel: &Cancel,
    progress: &mut dyn FnMut(Progress),
    report: &mut ExportReport,
) {
    let n = steps.len();
    for (i, step) in steps.iter().enumerate() {
        let number = i + 1;
        if cancel.is_cancelled() {
            report.cancelled = true;
            return;
        }
        progress(Progress {
            done: i,
            total: n,
            message: format!("Exporting {} of {n}: {}", number, label(step, number)),
        });
        // Cancel may arrive while the progress was being reported; stop before doing more work.
        if cancel.is_cancelled() {
            report.cancelled = true;
            return;
        }
        let image = match finished_image(store, step) {
            Ok(img) if options.include_text => with_text(project, step, number, img),
            other => other,
        };
        let image = match image {
            Ok(img) => img,
            Err(e) => {
                report.problem(
                    Some(step.id),
                    format!("{} was not exported: {e}", label(step, number)),
                );
                continue;
            }
        };
        let stem = file_stem(number, n, &step.title, options.names);
        match write_rgba(
            &options.dir,
            &stem,
            options.format,
            image.width,
            image.height,
            &image.rgba,
            existing,
        ) {
            Ok(path) => report.written.push(Written {
                path,
                width: image.width,
                height: image.height,
                step: Some(step.id),
            }),
            Err(e) => report.problem(
                Some(step.id),
                format!("{} could not be written: {e}", label(step, number)),
            ),
        }
    }
    progress(Progress {
        done: n,
        total: n,
        message: "Done".into(),
    });
}

#[allow(clippy::too_many_arguments)]
fn combined(
    store: &ProjectStore,
    project: &Project,
    steps: &[&Item],
    options: &ExportOptions,
    existing: ExistingFile,
    cancel: &Cancel,
    progress: &mut dyn FnMut(Progress),
    report: &mut ExportReport,
) {
    let n = steps.len();
    let total = n + 2;
    progress(Progress {
        done: 0,
        total,
        message: "Working out the layout".into(),
    });
    let sizes = store.item_sizes(project);
    let mut layout = match compute_layout(project, &sizes) {
        Ok(l) => l,
        Err(e) => {
            report.problem(None, e.to_string());
            return;
        }
    };
    if let Some(label) = &options.missing_label {
        layout.missing_label.clone_from(label);
    }
    for (i, step) in steps.iter().enumerate() {
        if !sizes.contains_key(&step.id) {
            report.problem(
                Some(step.id),
                format!("{}: the screenshot is missing or unreadable, so a grey placeholder is drawn in its place.", label(step, i + 1)),
            );
        }
    }
    let by_id: HashMap<ItemId, (usize, &Item)> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id, (i + 1, *s)))
        .collect();
    let mut drawn = 0usize;
    let mut failed: Vec<Problem> = Vec::new();
    let image = {
        let mut ask = |id: ItemId| -> Option<Image> {
            if cancel.is_cancelled() {
                return None;
            }
            let (number, step) = *by_id.get(&id)?;
            drawn += 1;
            progress(Progress {
                done: drawn,
                total,
                message: format!("Drawing {} of {n}: {}", drawn, label(step, number)),
            });
            match finished_image(store, step) {
                Ok(img) => Some(img),
                Err(e) => {
                    // Already reported above when the file is missing; report other failures here.
                    if sizes.contains_key(&id) {
                        failed.push(Problem {
                            step: Some(id),
                            message: format!("{} could not be drawn: {e}", label(step, number)),
                        });
                    }
                    None
                }
            }
        };
        render_layout_until(&layout, 1.0, &mut ask, &|| cancel.is_cancelled())
    };
    report.problems.extend(failed);
    if cancel.is_cancelled() || matches!(image, Err(ComposeError::Cancelled)) {
        report.cancelled = true;
        return;
    }
    let image = match image {
        Ok(img) => img,
        Err(e) => {
            report.problem(None, format!("Could not draw the combined image: {e}"));
            return;
        }
    };
    progress(Progress {
        done: n + 1,
        total,
        message: "Writing the file".into(),
    });
    let stem = combined_stem(project);
    match write_rgba(
        &options.dir,
        &stem,
        options.format,
        image.width,
        image.height,
        &image.rgba,
        existing,
    ) {
        Ok(path) => report.written.push(Written {
            path,
            width: image.width,
            height: image.height,
            step: None,
        }),
        Err(e) => report.problem(
            None,
            format!("The combined image could not be written: {e}"),
        ),
    }
    progress(Progress {
        done: total,
        total,
        message: "Done".into(),
    });
}

#[cfg(test)]
mod tests;
