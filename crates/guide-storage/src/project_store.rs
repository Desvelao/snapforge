//! A project on disk.
//!
//! ```text
//! my-project.<project extension>/
//!   project.json        the project (versioned, replaced atomically on every save)
//!   project.json.bak    the previous save, used to recover if project.json is damaged
//!   assets/             original screenshots, written once and never modified
//!   exports/            exported images
//! ```
//!
//! Only relative names are used, so the folder can be moved or copied anywhere.

use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
};

use guide_core::{
    asset_path_problem, ItemId, Project, ProjectError as ModelError, PROJECT_SCHEMA_VERSION,
};
use serde_json::Value;

use crate::{save_rgba, timestamped_stem, ImageFormat, OriginalImage};

pub const MANIFEST_FILE: &str = "project.json";
pub const BACKUP_FILE: &str = "project.json.bak";
pub const DAMAGED_FILE: &str = "project.json.damaged";
pub const ASSETS_DIR: &str = "assets";
pub const EXPORTS_DIR: &str = "exports";

/// Images above this many pixels are refused (about 480 MB decoded).
pub const MAX_IMAGE_PIXELS: u64 = 120_000_000;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("{0} already contains a project; refusing to overwrite it")]
    AlreadyExists(PathBuf),
    #[error("{0} is not a project (no project.json)")]
    NotAProject(PathBuf),
    #[error("the project file {path} is damaged: {reason}")]
    Corrupt { path: PathBuf, reason: String },
    #[error("the project file {path} cannot be used: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error("refusing to save an invalid project: {0}")]
    InvalidProject(ModelError),
    #[error("cannot read the image {path}: {reason}")]
    BadImage { path: PathBuf, reason: String },
    #[error(
        "{path} is too large ({width}x{height} pixels; the limit is {MAX_IMAGE_PIXELS} pixels)"
    )]
    TooLarge {
        path: PathBuf,
        width: u32,
        height: u32,
    },
    #[error("unsafe image path \"{path}\": {reason}")]
    UnsafePath { path: String, reason: String },
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    /// Writing, renaming or deleting failed.
    #[error("cannot change {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

/// For failures while *changing* something on disk.
fn io_err(path: &Path) -> impl FnOnce(io::Error) -> ProjectError {
    let path = path.to_path_buf();
    move |source| ProjectError::Io { path, source }
}

/// For failures while *reading* something from disk.
fn read_err(path: &Path) -> impl FnOnce(io::Error) -> ProjectError {
    let path = path.to_path_buf();
    move |source| ProjectError::Read { path, source }
}

/// A manifest bigger than this is not a project this app wrote (a 500-item project is a few
/// hundred kilobytes); refuse it instead of reading it all into memory.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

/// Turns a JSON parsing error into something a person can act on.
fn explain(e: &serde_json::Error) -> String {
    let text = e.to_string();
    if text.contains("unknown variant") {
        format!(
            "{text}. This project may have been made with a newer version that has \
             annotation types this version does not know"
        )
    } else {
        text
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueKind {
    MissingAsset,
    InvalidAsset,
    /// The path is fine but resolves outside the project (a symlink).
    AssetEscapes,
    InvalidAnnotations,
}

/// A problem with one item that does not stop the project from opening.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub item: ItemId,
    pub kind: IssueKind,
    pub message: String,
}

#[derive(Debug)]
pub struct Opened {
    pub store: ProjectStore,
    pub project: Project,
    pub issues: Vec<Issue>,
    /// Something the user should be told: a recovery, an upgrade.
    pub notice: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProjectStore {
    dir: PathBuf,
}

// ---- schema migration -----------------------------------------------------

struct Migration {
    from: u32,
    note: &'static str,
    apply: fn(&mut Value) -> Result<(), String>,
}

/// One entry per schema change, in order: `from` is the version it upgrades *from*.
/// There is none yet (the first released format is 1); see docs/project-format.md.
const MIGRATIONS: &[Migration] = &[];

/// Brings a manifest up to `target`, running each migration in turn.
/// Returns what was done. Newer-than-known versions are refused, never guessed at.
fn migrate_with(
    value: &mut Value,
    target: u32,
    table: &[Migration],
) -> Result<Vec<String>, String> {
    let found = value
        .get("schema_version")
        .ok_or("it has no schema_version")?;
    let mut version = found
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| format!("schema_version must be a whole number, found {found}"))?;
    if version == 0 {
        return Err("schema_version 0 is not valid".into());
    }
    if version > target {
        return Err(ModelError::NewerVersion(version).to_string());
    }
    let mut notes = Vec::new();
    while version < target {
        let step = table
            .iter()
            .find(|m| m.from == version)
            .ok_or_else(|| format!("no way to upgrade format {version} to {target}"))?;
        (step.apply)(value).map_err(|e| format!("upgrading from format {version} failed: {e}"))?;
        version += 1;
        value["schema_version"] = Value::from(version);
        notes.push(format!(
            "upgraded from format {} to {version}: {}",
            version - 1,
            step.note
        ));
    }
    Ok(notes)
}

// ---- helpers ----------------------------------------------------------------

/// Writes `bytes` to `path` so a reader never sees a partial file.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
    crate::atomic::write_atomic(path, bytes).map_err(io_err(path))
}

/// `project.json.damaged`, or `project.json.damaged.1`, `.2`... when earlier recoveries kept theirs.
fn free_damaged_name(dir: &Path) -> PathBuf {
    let first = dir.join(DAMAGED_FILE);
    if !first.exists() {
        return first;
    }
    (1..)
        .map(|n| dir.join(format!("{DAMAGED_FILE}.{n}")))
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

/// Reads, upgrades and validates one manifest file.
fn read_manifest(path: &Path) -> Result<(Project, Vec<String>), ProjectError> {
    let not_found_or_read = |e: io::Error| {
        if e.kind() == io::ErrorKind::NotFound {
            ProjectError::NotAProject(path.parent().unwrap_or(path).to_path_buf())
        } else {
            read_err(path)(e)
        }
    };
    let len = fs::metadata(path).map_err(not_found_or_read)?.len();
    if len > MAX_MANIFEST_BYTES {
        return Err(ProjectError::Invalid {
            path: path.to_path_buf(),
            reason: format!(
                "it is too large ({} MB; the limit is {} MB), so it is not a project this app wrote",
                len / (1024 * 1024),
                MAX_MANIFEST_BYTES / (1024 * 1024)
            ),
        });
    }
    let bytes = fs::read(path).map_err(not_found_or_read)?;
    let text = std::str::from_utf8(&bytes).map_err(|e| ProjectError::Corrupt {
        path: path.to_path_buf(),
        reason: format!(
            "it is not valid UTF-8 text (bad byte at position {})",
            e.valid_up_to()
        ),
    })?;
    // Some editors put a byte-order mark at the start of a UTF-8 file; JSON parsers reject it.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut value: Value = serde_json::from_str(text).map_err(|e| ProjectError::Corrupt {
        path: path.to_path_buf(),
        reason: explain(&e),
    })?;
    let notes = migrate_with(&mut value, PROJECT_SCHEMA_VERSION, MIGRATIONS).map_err(|reason| {
        ProjectError::Invalid {
            path: path.to_path_buf(),
            reason,
        }
    })?;
    if value.get("steps").is_some() && value.get("items").is_none() {
        return Err(ProjectError::Invalid {
            path: path.to_path_buf(),
            reason: "it was saved with an older layout of project.json (a guide with \"steps\") \
                     that this version can no longer read"
                .into(),
        });
    }
    let mut project: Project =
        serde_json::from_value(value).map_err(|e| ProjectError::Corrupt {
            path: path.to_path_buf(),
            reason: explain(&e),
        })?;
    project.repair();
    let mut notes = notes;
    notes.extend(project.sanitize());
    project.validate().map_err(|e| ProjectError::Invalid {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    Ok((project, notes))
}

/// A newer-format file is valid, just not readable here; it must not be "recovered" over.
fn is_newer_version(e: &ProjectError) -> bool {
    matches!(e, ProjectError::Invalid { reason, .. } if reason.contains("newer version"))
}

impl ProjectStore {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Starts a new project folder holding `project`. Fails, touching nothing, if one exists there.
    pub fn create(dir: &Path, project: &Project) -> Result<Self, ProjectError> {
        if dir.join(MANIFEST_FILE).exists() {
            return Err(ProjectError::AlreadyExists(dir.to_path_buf()));
        }
        project.validate().map_err(ProjectError::InvalidProject)?;
        let store = Self {
            dir: dir.to_path_buf(),
        };
        for sub in [ASSETS_DIR, EXPORTS_DIR] {
            let path = store.project_subdir(sub)?;
            fs::create_dir_all(&path).map_err(io_err(&path))?;
        }
        store.save(project)?;
        Ok(store)
    }

    /// Opens a project. A damaged `project.json` is recovered from `project.json.bak`
    /// (the damaged file is kept as `project.json.damaged`). Missing or unreadable
    /// images do not stop the project opening; they are returned as [`Issue`]s.
    pub fn open(dir: &Path) -> Result<Opened, ProjectError> {
        let manifest = dir.join(MANIFEST_FILE);
        let backup = dir.join(BACKUP_FILE);
        let mut notice = None;
        let (project, notes) = match read_manifest(&manifest) {
            Ok(r) => r,
            Err(e) if is_newer_version(&e) => return Err(e),
            Err(first) => match read_manifest(&backup) {
                Ok(r) if manifest.exists() || backup.exists() => {
                    // Keep the damaged file under a name nothing else uses, then put the backup in
                    // place with one atomic replace: at no moment is there no readable manifest
                    // except the damaged one, and an earlier damaged file is never overwritten.
                    let damaged = free_damaged_name(dir);
                    let kept = if manifest.exists() {
                        fs::copy(&manifest, &damaged).map_err(io_err(&damaged))?;
                        damaged
                            .file_name()
                            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                    } else {
                        DAMAGED_FILE.to_string()
                    };
                    let bytes = fs::read(&backup).map_err(read_err(&backup))?;
                    atomic_write(&manifest, &bytes)?;
                    notice = Some(format!(
                        "{MANIFEST_FILE} was unreadable ({first}). Restored the previous save from {BACKUP_FILE}; \
                         the last changes may be missing. The damaged file was kept as {kept}."
                    ));
                    r
                }
                _ => return Err(first),
            },
        };
        if !notes.is_empty() {
            let msg = notes.join("; ");
            notice = Some(notice.map_or(msg.clone(), |n| format!("{n} {msg}")));
        }
        let store = Self {
            dir: dir.to_path_buf(),
        };
        let issues = store.check_assets(&project);
        Ok(Opened {
            store,
            project,
            issues,
            notice,
        })
    }

    /// Writes the manifest atomically, keeping the previous one as `project.json.bak`.
    /// An invalid project (unsafe paths, duplicate ids) is refused before anything is written.
    pub fn save(&self, project: &Project) -> Result<(), ProjectError> {
        project.validate().map_err(ProjectError::InvalidProject)?;
        let manifest = self.dir.join(MANIFEST_FILE);
        if manifest.exists() {
            // Only keep a previous save that is itself readable, so a damaged file never replaces a good backup.
            if read_manifest(&manifest).is_ok() {
                let bytes = fs::read(&manifest).map_err(read_err(&manifest))?;
                atomic_write(&self.dir.join(BACKUP_FILE), &bytes)?;
            }
        }
        let mut json =
            serde_json::to_string_pretty(project).map_err(|e| ProjectError::Corrupt {
                path: manifest.clone(),
                reason: e.to_string(),
            })?;
        json.push('\n');
        atomic_write(&manifest, json.as_bytes())
    }

    /// Resolves `assets/<name>` to a real path inside the project, or says why it cannot be used.
    /// Beyond the syntactic check, the resolved file must really live inside the project
    /// folder, so a symlink pointing elsewhere is refused.
    pub fn asset_path(&self, rel: &str) -> Result<PathBuf, ProjectError> {
        if let Some(reason) = asset_path_problem(rel) {
            return Err(ProjectError::UnsafePath {
                path: rel.to_string(),
                reason: reason.to_string(),
            });
        }
        let path = self.dir.join(rel);
        let root = fs::canonicalize(&self.dir).map_err(read_err(&self.dir))?;
        let outside = |real: &Path| ProjectError::UnsafePath {
            path: rel.to_string(),
            reason: format!("resolves to {} outside the project folder", real.display()),
        };
        // The resolved path is returned, so what the caller opens is the file that was checked.
        match fs::canonicalize(&path) {
            Ok(real) if real.starts_with(&root) => Ok(real),
            Ok(real) => Err(outside(&real)),
            // A missing file is not unsafe; the caller reports it as missing. Its folder must
            // still be inside the project, or a link in its place would send a write elsewhere.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let parent = path.parent().unwrap_or(&self.dir);
                match fs::canonicalize(parent) {
                    Ok(real) if real.starts_with(&root) => {
                        Ok(real.join(path.file_name().unwrap_or_default()))
                    }
                    Ok(real) => Err(outside(&real)),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(path),
                    Err(e) => Err(read_err(parent)(e)),
                }
            }
            Err(e) => Err(read_err(&path)(e)),
        }
    }

    /// `assets/` or `exports/`, checked to be a real folder of this project and not a link that
    /// would send files somewhere else. A folder that does not exist yet is fine.
    fn project_subdir(&self, sub: &str) -> Result<PathBuf, ProjectError> {
        let dir = self.dir.join(sub);
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.is_dir() => Ok(dir),
            Ok(_) => Err(ProjectError::UnsafePath {
                path: sub.to_string(),
                reason: "it must be a folder inside the project, not a link or a file".into(),
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(dir),
            Err(e) => Err(read_err(&dir)(e)),
        }
    }

    fn check_assets(&self, project: &Project) -> Vec<Issue> {
        let mut issues = Vec::new();
        for step in &project.items {
            let mut issue = |kind, message: String| {
                issues.push(Issue {
                    item: step.id,
                    kind,
                    message,
                })
            };
            let name = || {
                format!(
                    "\"{}\"",
                    if step.title.is_empty() {
                        "untitled item"
                    } else {
                        &step.title
                    }
                )
            };
            let path = match self.asset_path(&step.image) {
                Ok(p) => p,
                Err(e) => {
                    issue(IssueKind::AssetEscapes, format!("Item {}: {e}", name()));
                    continue;
                }
            };
            if !path.exists() {
                issue(
                    IssueKind::MissingAsset,
                    format!("Item {}: the screenshot {} is missing.", name(), step.image),
                );
                continue;
            }
            match image::image_dimensions(&path) {
                Err(e) => issue(
                    IssueKind::InvalidAsset,
                    format!(
                        "Item {}: {} is not a readable image ({e}).",
                        name(),
                        step.image
                    ),
                ),
                Ok((w, h)) if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS => issue(
                    IssueKind::InvalidAsset,
                    format!(
                        "Item {}: {} is too large ({w}x{h} pixels).",
                        name(),
                        step.image
                    ),
                ),
                Ok(dims) => {
                    if let Err(why) = step.annotations.validate(dims) {
                        issue(
                            IssueKind::InvalidAnnotations,
                            format!(
                                "Item {}: its annotations do not fit {}: {why}.",
                                name(),
                                step.image
                            ),
                        );
                    }
                }
            }
        }
        issues
    }

    /// The finished screenshot size of every item whose image can be read: the crop if the
    /// item has one, otherwise the original size. Reads only image headers, so it is cheap
    /// even for hundreds of items. Items with a missing or unsafe image are simply absent.
    pub fn item_sizes(&self, project: &Project) -> HashMap<ItemId, (u32, u32)> {
        project
            .items
            .iter()
            .filter_map(|step| {
                let path = self.asset_path(&step.image).ok()?;
                let (w, h) = image::image_dimensions(path).ok()?;
                let size = step.annotations.crop.map_or((w, h), |c| (c.w, c.h));
                Some((step.id, size))
            })
            .collect()
    }

    /// Stores a new original screenshot as `assets/shot-<timestamp>.png` (never overwriting
    /// an existing file) and returns its project-relative path.
    pub fn add_asset_rgba(
        &self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<String, ProjectError> {
        if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
            return Err(ProjectError::TooLarge {
                path: self.dir.join(ASSETS_DIR),
                width,
                height,
            });
        }
        let assets = self.project_subdir(ASSETS_DIR)?;
        let path = save_rgba(
            &assets,
            &format!("shot-{}", timestamped_stem()),
            ImageFormat::Png,
            width,
            height,
            rgba,
        )
        .map_err(|e| ProjectError::BadImage {
            path: assets,
            reason: e.to_string(),
        })?;
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        Ok(format!("{ASSETS_DIR}/{name}"))
    }

    /// Imports any supported image file (PNG, JPEG, WebP): decodes it and stores it
    /// as a PNG asset. The source file is not modified. Returns the path and size.
    pub fn import_image(&self, src: &Path) -> Result<(String, u32, u32), ProjectError> {
        let bad = |reason: String| ProjectError::BadImage {
            path: src.to_path_buf(),
            reason,
        };
        let (w, h) = image::image_dimensions(src).map_err(|e| bad(e.to_string()))?;
        if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
            return Err(ProjectError::TooLarge {
                path: src.to_path_buf(),
                width: w,
                height: h,
            });
        }
        let img = image::open(src).map_err(|e| bad(e.to_string()))?.to_rgba8();
        let rel = self.add_asset_rgba(w, h, img.as_raw())?;
        Ok((rel, w, h))
    }

    pub fn load_asset(&self, rel: &str) -> Result<OriginalImage, ProjectError> {
        let path = self.asset_path(rel)?;
        let img = image::open(&path)
            .map_err(|e| ProjectError::BadImage {
                path: path.clone(),
                reason: e.to_string(),
            })?
            .to_rgba8();
        Ok(OriginalImage {
            width: img.width(),
            height: img.height(),
            rgba: img.into_raw(),
        })
    }

    /// Deletes screenshots in `assets/` that no item refers to any more. Only files this app
    /// itself created (`shot-*.png`) are ever removed, symlinks are skipped, and anything
    /// still referenced (for example by a duplicated item) is kept. Returns what was removed.
    pub fn prune_unreferenced_assets(
        &self,
        project: &Project,
    ) -> Result<Vec<String>, ProjectError> {
        let keep: HashSet<&str> = project.referenced_assets().into_iter().collect();
        let assets = self.project_subdir(ASSETS_DIR)?;
        let mut removed = Vec::new();
        let entries = match fs::read_dir(&assets) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(removed),
            Err(e) => return Err(read_err(&assets)(e)),
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_ours = name.starts_with("shot-") && name.ends_with(".png");
            let is_plain_file = entry.path().symlink_metadata().is_ok_and(|m| m.is_file());
            let rel = format!("{ASSETS_DIR}/{name}");
            if is_ours && is_plain_file && !keep.contains(rel.as_str()) {
                fs::remove_file(entry.path()).map_err(io_err(&entry.path()))?;
                removed.push(rel);
            }
        }
        removed.sort();
        Ok(removed)
    }
}

#[cfg(test)]
mod tests;
