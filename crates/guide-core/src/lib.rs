//! Domain models shared by every component (no UI, no I/O).

mod annotation;
mod guide;
mod history;
mod mode;
mod project;
mod shortcut;

pub use annotation::{
    arrow_head_len, Annotation, AnnotationId, Kind, ObscureMode, PixelRect, Point, RectF, Rgba,
    Style,
};
pub use guide::{
    CompositionSettings, DescriptionPlacement, ImageAlign, ImageScaling, Layout, StepLayout,
};
pub use history::{Command, Document, Editor, DEFAULT_UNDO_DEPTH, DOCUMENT_VERSION};
pub use mode::ModeChoice;
pub use project::{
    asset_path_problem, new_project_id, slug, GuideSettings, Item, ItemId, NewItem, Project,
    ProjectError, ProjectKind, PROJECT_SCHEMA_VERSION,
};
pub use shortcut::{canonical_key, find_duplicate, Action, Shortcut, ShortcutError, KEY_NAMES};

/// Application display name, from `app.toml` (read by `build.rs`).
pub const APP_NAME: &str = env!("APP_DISPLAY_NAME");

/// Application version, from `app.toml` (read by `build.rs`); `Cargo.toml` must match.
pub const APP_VERSION: &str = env!("APP_VERSION");

/// Extension of a project folder, without the dot, from `app.toml` (read by `build.rs`).
pub const PROJECT_EXTENSION: &str = env!("PROJECT_EXTENSION");

/// Extension of a saved edit folder, without the dot, from `app.toml` (read by `build.rs`).
pub const EDIT_EXTENSION: &str = env!("EDIT_EXTENSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_display_name_comes_from_the_app_file() {
        assert!(!APP_NAME.trim().is_empty());
        assert!(!APP_NAME.contains('"'));
    }
}
