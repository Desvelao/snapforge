//! Project manifest persistence, path validation and atomic writes.

mod atomic;
mod edit_bundle;
mod image_out;
mod project_store;
mod settings;

pub use edit_bundle::{BundleError, EditBundle, OriginalImage, ANNOTATIONS_FILE, ORIGINAL_FILE};
pub use image_out::{
    save_rgba, save_rgba_at, timestamped_stem, write_rgba, ExistingFile, ImageFormat, SaveError,
};
pub use project_store::{
    Issue, IssueKind, Opened, ProjectError, ProjectStore, ASSETS_DIR, BACKUP_FILE, DAMAGED_FILE,
    EXPORTS_DIR, MANIFEST_FILE, MAX_IMAGE_PIXELS, MAX_MANIFEST_BYTES,
};
pub use settings::{
    remember_recent, ClickCaptureOptions, EditorOptions, Loaded, Profile, ProfileError, Settings,
    SettingsError, ShortcutSettings, ThemeChoice, CURRENT_VERSION, DEFAULT_PROFILE,
    MAX_CLICK_SHOTS, MAX_PROFILES, MAX_PROFILE_NAME, MAX_RECENT,
};
