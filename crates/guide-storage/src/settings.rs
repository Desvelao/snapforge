use std::{
    fs, io,
    path::{Path, PathBuf},
};

use guide_core::{find_duplicate, Action, ModeChoice, ObscureMode, Rgba, Shortcut};
use serde::{Deserialize, Serialize};

use crate::ImageFormat;

/// Bumped when a change to the file would confuse older versions.
pub const CURRENT_VERSION: u32 = 1;

/// Shortcuts as the user typed them; parsed and validated on load.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutSettings {
    pub capture_full_screen: String,
    pub capture_region: String,
    pub capture_window: String,
    pub toggle_click_capture: String,
}

impl Default for ShortcutSettings {
    fn default() -> Self {
        Self {
            capture_full_screen: Action::CaptureFullScreen.default_shortcut().to_string(),
            capture_region: Action::CaptureRegion.default_shortcut().to_string(),
            capture_window: Action::CaptureWindow.default_shortcut().to_string(),
            toggle_click_capture: Action::ToggleClickCapture.default_shortcut().to_string(),
        }
    }
}

impl ShortcutSettings {
    pub fn get(&self, action: Action) -> &str {
        match action {
            Action::CaptureFullScreen => &self.capture_full_screen,
            Action::CaptureRegion => &self.capture_region,
            Action::CaptureWindow => &self.capture_window,
            Action::ToggleClickCapture => &self.toggle_click_capture,
        }
    }

    pub fn set(&mut self, action: Action, shortcut: &Shortcut) {
        let slot = match action {
            Action::CaptureFullScreen => &mut self.capture_full_screen,
            Action::CaptureRegion => &mut self.capture_region,
            Action::CaptureWindow => &mut self.capture_window,
            Action::ToggleClickCapture => &mut self.toggle_click_capture,
        };
        *slot = shortcut.to_string();
    }

    /// Every action with its parsed shortcut. Invalid entries are skipped; use
    /// [`Settings::sanitize`] to repair them first.
    pub fn bindings(&self) -> Vec<(Action, Shortcut)> {
        Action::ALL
            .iter()
            .filter_map(|a| Shortcut::parse(self.get(*a)).ok().map(|s| (*a, s)))
            .collect()
    }
}

/// Which colour scheme the window uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    /// Follow the desktop when it says whether it is light or dark; otherwise dark.
    #[default]
    System,
    Light,
    Dark,
}

/// How many recent projects / edits are remembered.
pub const MAX_RECENT: usize = 10;
/// Upper bound for the click-capture shot limit.
pub const MAX_CLICK_SHOTS: u32 = 1000;
/// How many profiles can be kept, and how long a profile name may be.
pub const MAX_PROFILES: usize = 20;
pub const MAX_PROFILE_NAME: usize = 40;
/// Name of the profile created when none exist.
pub const DEFAULT_PROFILE: &str = "Default";

/// Click capture settings kept in a profile.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClickCaptureOptions {
    /// Put a marker annotation at the click on each new guide step.
    pub marker: bool,
    /// Stop after this many shots; 0 means no limit.
    pub max: u32,
}

/// The annotation editor's tool settings kept in a profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorOptions {
    pub color: Rgba,
    pub width: f32,
    pub highlighter_width: f32,
    pub fill_on: bool,
    pub fill: Rgba,
    pub text_size: f32,
    pub bold: bool,
    pub obscure_mode: ObscureMode,
    pub obscure_strength: u32,
    pub marker_radius: f32,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            color: Rgba::RED,
            width: 4.0,
            highlighter_width: 22.0,
            fill_on: false,
            fill: Rgba::WHITE,
            text_size: 28.0,
            bold: false,
            obscure_mode: ObscureMode::Pixelate,
            obscure_strength: 14,
            marker_radius: 16.0,
        }
    }
}

impl EditorOptions {
    /// Pulls absurd or non-finite sizes back into a usable range.
    fn clamp(&mut self) {
        for (v, lo, hi) in [
            (&mut self.width, 1.0, 100.0),
            (&mut self.highlighter_width, 1.0, 200.0),
            (&mut self.text_size, 6.0, 400.0),
            (&mut self.marker_radius, 4.0, 200.0),
        ] {
            *v = if v.is_finite() { v.clamp(lo, hi) } else { lo };
        }
        self.obscure_strength = self.obscure_strength.clamp(1, 100);
    }
}

/// A named set of tool settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub click_capture_options: ClickCaptureOptions,
    pub editor_options: EditorOptions,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: DEFAULT_PROFILE.to_string(),
            click_capture_options: ClickCaptureOptions::default(),
            editor_options: EditorOptions::default(),
        }
    }
}

/// Why a profile change was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("the name is empty")]
    EmptyName,
    #[error("a profile with that name already exists")]
    Duplicate,
    #[error("there is no profile with that name")]
    Missing,
    #[error("at least one profile must remain")]
    LastProfile,
    #[error("too many profiles")]
    TooMany,
}

/// Puts `path` at the front of `list`, removing an earlier copy and keeping at most [`MAX_RECENT`].
pub fn remember_recent(list: &mut Vec<String>, path: &str) {
    let path = path.trim();
    if path.is_empty() {
        return;
    }
    list.retain(|p| p != path);
    list.insert(0, path.to_string());
    list.truncate(MAX_RECENT);
}

/// A settings file bigger than this is not one; it is set aside instead of read.
const MAX_SETTINGS_BYTES: u64 = 4 * 1024 * 1024;

/// `settings.json.bad`, or `.bad.1`, `.bad.2`... when earlier damaged files are kept there.
fn free_bad_name(path: &Path) -> PathBuf {
    let first = path.with_extension("json.bad");
    if !first.exists() {
        return first;
    }
    (1..)
        .map(|n| path.with_extension(format!("json.bad.{n}")))
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

/// For a file that is JSON but does not fit [`Settings`] as a whole: keeps every top-level value
/// that fits on its own and returns the names of the ones that do not. `None` when it is not a
/// JSON object at all.
fn lenient(text: &str) -> Option<(Settings, Vec<String>)> {
    let serde_json::Value::Object(map) = serde_json::from_str(text).ok()? else {
        return None;
    };
    let mut good = serde_json::Map::new();
    let mut ignored = Vec::new();
    for (key, value) in map {
        let mut one = serde_json::Map::new();
        one.insert(key.clone(), value.clone());
        if serde_json::from_value::<Settings>(serde_json::Value::Object(one)).is_ok() {
            good.insert(key, value);
        } else {
            ignored.push(key);
        }
    }
    let settings = serde_json::from_value(serde_json::Value::Object(good)).ok()?;
    Some((settings, ignored))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub out_dir: String,
    pub format: ImageFormat,
    pub delay_secs: u32,
    pub include_cursor: bool,
    pub mode: ModeChoice,
    pub shortcuts: ShortcutSettings,
    /// Language code such as `de`; empty means "use the desktop's language".
    pub language: String,
    pub theme: ThemeChoice,
    /// Most recently opened project folders, newest first.
    pub recent_projects: Vec<String>,
    /// Most recently saved or opened edit folders, newest first.
    pub recent_edits: Vec<String>,
    /// Click capture: put a marker annotation at the click on each new guide step.
    pub click_marker: bool,
    /// Click capture: stop after this many shots; 0 means no limit.
    pub click_max: u32,
    /// Named tool settings; the active one is applied to click capture and the editor.
    /// Absent in files from before profiles existed; `sanitize` then seeds one from `click_*`.
    #[serde(default = "Vec::new")]
    pub profiles: Vec<Profile>,
    /// Name of the profile selected last; restored on start.
    #[serde(default = "Option::default")]
    pub active_profile: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let out_dir = std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Pictures").join(guide_core::APP_NAME))
            .unwrap_or_else(|| PathBuf::from("."))
            .to_string_lossy()
            .into_owned();
        Self {
            version: CURRENT_VERSION,
            out_dir,
            format: ImageFormat::default(),
            delay_secs: 0,
            include_cursor: false,
            mode: ModeChoice::default(),
            shortcuts: ShortcutSettings::default(),
            language: String::new(),
            theme: ThemeChoice::default(),
            recent_projects: Vec::new(),
            recent_edits: Vec::new(),
            click_marker: false,
            click_max: 0,
            profiles: vec![Profile::default()],
            active_profile: Some(DEFAULT_PROFILE.to_string()),
        }
    }
}

impl Settings {
    /// Repairs values that cannot work (unparsable or duplicated shortcuts,
    /// absurd delay) by resetting them to defaults. Returns one message per repair.
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        for a in Action::ALL {
            if let Err(e) = Shortcut::parse(self.shortcuts.get(a)) {
                notes.push(format!(
                    "Shortcut \"{}\" for \"{}\" is invalid ({e}); reset to {}",
                    self.shortcuts.get(a),
                    a.label(),
                    a.default_shortcut()
                ));
                self.shortcuts.set(a, &a.default_shortcut());
            }
        }
        // Resetting can itself collide with another entry, so repeat until stable.
        while let Some((_, second)) = find_duplicate(&self.shortcuts.bindings()) {
            notes.push(format!(
                "\"{}\" shared its shortcut with another action; reset to {}",
                second.label(),
                second.default_shortcut()
            ));
            self.shortcuts.set(second, &second.default_shortcut());
            if notes.len() > Action::ALL.len() * 2 {
                break; // defaults are unique, so this cannot loop forever; belt and braces
            }
        }
        for list in [&mut self.recent_projects, &mut self.recent_edits] {
            let mut seen = std::collections::HashSet::new();
            list.retain(|p| !p.trim().is_empty() && seen.insert(p.clone()));
            list.truncate(MAX_RECENT);
        }
        if self.delay_secs > 60 {
            notes.push(format!(
                "Delay of {}s is too long; reset to 60s",
                self.delay_secs
            ));
            self.delay_secs = 60;
        }
        if self.click_max > MAX_CLICK_SHOTS {
            notes.push(format!(
                "Click capture limit of {} is too high; reset to {MAX_CLICK_SHOTS}",
                self.click_max
            ));
            self.click_max = MAX_CLICK_SHOTS;
        }
        self.sanitize_profiles(&mut notes);
        notes
    }

    fn sanitize_profiles(&mut self, notes: &mut Vec<String>) {
        let mut seen = std::collections::HashSet::new();
        self.profiles.retain_mut(|p| {
            p.name = p.name.trim().chars().take(MAX_PROFILE_NAME).collect();
            !p.name.is_empty() && seen.insert(p.name.to_lowercase())
        });
        self.profiles.truncate(MAX_PROFILES);
        for p in &mut self.profiles {
            p.click_capture_options.max = p.click_capture_options.max.min(MAX_CLICK_SHOTS);
            p.editor_options.clamp();
        }
        if self.profiles.is_empty() {
            // First run with profiles: keep the click options the user already had.
            self.profiles.push(Profile {
                click_capture_options: ClickCaptureOptions {
                    marker: self.click_marker,
                    max: self.click_max,
                },
                ..Profile::default()
            });
        }
        let known = self
            .active_profile
            .as_ref()
            .is_some_and(|n| self.profiles.iter().any(|p| &p.name == n));
        if !known {
            if self.active_profile.is_some() {
                notes.push("The last used profile no longer exists; using the first one".into());
            }
            self.active_profile = Some(self.profiles[0].name.clone());
        }
        // Older builds read these two, so keep them in step with the active profile.
        let c = self.active().click_capture_options.clone();
        (self.click_marker, self.click_max) = (c.marker, c.max);
    }

    /// The selected profile. `sanitize` guarantees one exists.
    pub fn active(&self) -> &Profile {
        let name = self.active_profile.as_deref();
        self.profiles
            .iter()
            .find(|p| Some(p.name.as_str()) == name)
            .or(self.profiles.first())
            .expect("sanitize keeps at least one profile")
    }

    pub fn active_mut(&mut self) -> &mut Profile {
        let name = self.active_profile.clone();
        let i = self
            .profiles
            .iter()
            .position(|p| Some(&p.name) == name.as_ref())
            .unwrap_or(0);
        &mut self.profiles[i]
    }

    fn checked_name(&self, name: &str, except: Option<&str>) -> Result<String, ProfileError> {
        let name: String = name.trim().chars().take(MAX_PROFILE_NAME).collect();
        if name.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        let taken = self.profiles.iter().any(|p| {
            p.name.to_lowercase() == name.to_lowercase() && Some(p.name.as_str()) != except
        });
        if taken {
            return Err(ProfileError::Duplicate);
        }
        Ok(name)
    }

    /// Adds a copy of the active profile under `name` and selects it.
    pub fn add_profile(&mut self, name: &str) -> Result<(), ProfileError> {
        let name = self.checked_name(name, None)?;
        if self.profiles.len() >= MAX_PROFILES {
            return Err(ProfileError::TooMany);
        }
        let mut p = self.active().clone();
        p.name = name.clone();
        self.profiles.push(p);
        self.active_profile = Some(name);
        Ok(())
    }

    pub fn rename_profile(&mut self, from: &str, to: &str) -> Result<(), ProfileError> {
        let to = self.checked_name(to, Some(from))?;
        let p = self
            .profiles
            .iter_mut()
            .find(|p| p.name == from)
            .ok_or(ProfileError::Missing)?;
        p.name = to.clone();
        if self.active_profile.as_deref() == Some(from) {
            self.active_profile = Some(to);
        }
        Ok(())
    }

    /// Removes a profile; the first remaining one becomes active if it was the selected one.
    pub fn delete_profile(&mut self, name: &str) -> Result<(), ProfileError> {
        let i = self
            .profiles
            .iter()
            .position(|p| p.name == name)
            .ok_or(ProfileError::Missing)?;
        if self.profiles.len() == 1 {
            return Err(ProfileError::LastProfile);
        }
        self.profiles.remove(i);
        if self.active_profile.as_deref() == Some(name) {
            self.active_profile = Some(self.profiles[0].name.clone());
        }
        Ok(())
    }

    pub fn select_profile(&mut self, name: &str) -> Result<(), ProfileError> {
        if !self.profiles.iter().any(|p| p.name == name) {
            return Err(ProfileError::Missing);
        }
        self.active_profile = Some(name.to_string());
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("no home directory: set HOME or XDG_CONFIG_HOME")]
    NoConfigDir,
    #[error("cannot write {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("cannot encode settings: {0}")]
    Encode(String),
}

/// Result of [`Settings::load`]: always usable settings, plus a user-facing
/// notice when something had to be repaired or could not be read.
#[derive(Debug)]
pub struct Loaded {
    pub settings: Settings,
    pub notice: Option<String>,
}

impl Settings {
    /// `$XDG_CONFIG_HOME/snapforge/settings.json`, else `~/.config/snapforge/settings.json`.
    pub fn default_path() -> Result<PathBuf, SettingsError> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .ok_or(SettingsError::NoConfigDir)?;
        Ok(base.join("snapforge").join("settings.json"))
    }

    /// Never fails. A missing file gives defaults silently. A file that is not JSON (or is far
    /// too large) is moved to `<name>.bad` (`.bad.1`, ... if that exists) so nothing is lost, and
    /// defaults are used. A file that is JSON but has a value this version cannot read (a newer
    /// version's option) keeps everything else and resets only that value.
    pub fn load(path: &Path) -> Loaded {
        let too_large = fs::metadata(path).is_ok_and(|m| m.len() > MAX_SETTINGS_BYTES);
        let read = if too_large {
            Err(io::Error::other(
                "the file is far larger than any settings file",
            ))
        } else {
            fs::read_to_string(path)
        };
        let text = match read {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Loaded {
                    settings: Settings::default(),
                    notice: None,
                };
            }
            Err(e) if too_large => {
                let bad = free_bad_name(path);
                let moved = fs::rename(path, &bad).is_ok();
                return Loaded {
                    settings: Settings::default(),
                    notice: Some(format!(
                        "Could not read settings {}: {e}. Using defaults.{}",
                        path.display(),
                        if moved {
                            format!(" The file was kept as {}.", bad.display())
                        } else {
                            String::new()
                        }
                    )),
                };
            }
            Err(e) => {
                return Loaded {
                    settings: Settings::default(),
                    notice: Some(format!(
                        "Could not read settings {}: {e}. Using defaults.",
                        path.display()
                    )),
                };
            }
        };
        let (parsed, ignored) = match serde_json::from_str::<Settings>(&text) {
            Ok(s) => (Ok(s), Vec::new()),
            Err(e) => match lenient(&text) {
                Some((s, ignored)) => (Ok(s), ignored),
                None => (Err(e), Vec::new()),
            },
        };
        match parsed {
            Ok(mut settings) => {
                let mut notes = settings.sanitize();
                if !ignored.is_empty() {
                    notes.push(format!(
                        "These settings have a value this version does not understand and were reset: {}",
                        ignored.join(", ")
                    ));
                }
                if settings.version > CURRENT_VERSION {
                    // Saving writes only what this version knows, so keep the original once.
                    let newer = path.with_extension(format!("json.v{}", settings.version));
                    let kept = if newer.exists() || fs::copy(path, &newer).is_ok() {
                        format!(" The file was kept as {}.", newer.display())
                    } else {
                        String::new()
                    };
                    notes.push(format!(
                        "Settings were saved by a newer version (format {}); unknown values are ignored.{kept}",
                        settings.version
                    ));
                }
                settings.version = CURRENT_VERSION;
                Loaded {
                    settings,
                    notice: (!notes.is_empty()).then(|| notes.join(". ")),
                }
            }
            Err(e) => {
                let bad = free_bad_name(path);
                let moved = fs::rename(path, &bad).is_ok();
                let kept = if moved {
                    format!("The damaged file was kept as {}.", bad.display())
                } else {
                    "The damaged file could not be moved aside and will be overwritten.".into()
                };
                Loaded {
                    settings: Settings::default(),
                    notice: Some(format!(
                        "Settings file is damaged ({e}). Using defaults. {kept}"
                    )),
                }
            }
        }
    }

    /// Writes the file atomically: a temp file in the same directory is
    /// flushed to disk, then renamed over the target.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let io_err = |p: &Path| {
            let p = p.to_path_buf();
            move |source| SettingsError::Io { path: p, source }
        };
        let json =
            serde_json::to_string_pretty(self).map_err(|e| SettingsError::Encode(e.to_string()))?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(dir).map_err(io_err(dir))?;
        let mut body = json.into_bytes();
        body.push(b'\n');
        crate::atomic::write_atomic(path, &body).map_err(io_err(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("snapforge-settings-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn custom() -> Settings {
        Settings {
            out_dir: "/tmp/shots".into(),
            format: ImageFormat::Jpeg,
            delay_secs: 5,
            include_cursor: true,
            mode: ModeChoice::Window,
            shortcuts: ShortcutSettings {
                capture_region: "Ctrl+Alt+R".into(),
                ..ShortcutSettings::default()
            },
            ..Settings::default()
        }
    }

    #[test]
    fn missing_file_gives_defaults_without_notice() {
        let l = Settings::load(&dir("missing").join("settings.json"));
        assert_eq!(l.settings, Settings::default());
        assert_eq!(l.notice, None);
    }

    #[test]
    fn save_then_load_round_trips_and_creates_directories() {
        let d = dir("roundtrip");
        let path = d.join("nested").join("settings.json");
        custom().save(&path).unwrap();
        let l = Settings::load(&path);
        assert_eq!(l.settings, custom());
        assert_eq!(l.notice, None);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_overwrites_and_leaves_no_temp_file() {
        let d = dir("overwrite");
        let path = d.join("settings.json");
        Settings::default().save(&path).unwrap();
        custom().save(&path).unwrap();
        assert_eq!(Settings::load(&path).settings, custom());
        let names: Vec<_> = fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("settings.json")]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let d = dir("fields");
        fs::create_dir_all(&d).unwrap();
        let path = d.join("settings.json");
        fs::write(&path, r#"{"delay_secs": 3, "future_option": {"x": 1}}"#).unwrap();
        let l = Settings::load(&path);
        assert_eq!(l.settings.delay_secs, 3);
        assert_eq!(l.settings.format, ImageFormat::Png);
        assert_eq!(l.notice, None);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn corrupt_file_is_kept_as_bad_and_defaults_are_used() {
        let d = dir("corrupt");
        fs::create_dir_all(&d).unwrap();
        let path = d.join("settings.json");
        fs::write(&path, "{ not json").unwrap();
        let l = Settings::load(&path);
        assert_eq!(l.settings, Settings::default());
        assert!(l.notice.as_deref().unwrap().contains("settings.json.bad"));
        assert_eq!(
            fs::read_to_string(d.join("settings.json.bad")).unwrap(),
            "{ not json"
        );
        assert!(!path.exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_second_damaged_file_does_not_replace_the_first_one_kept() {
        let d = dir("two-bad");
        fs::create_dir_all(&d).unwrap();
        let path = d.join("settings.json");
        fs::write(&path, "first {").unwrap();
        Settings::load(&path);
        fs::write(&path, "second {").unwrap();
        let l = Settings::load(&path);
        assert!(l.notice.unwrap().contains("settings.json.bad.1"));
        assert_eq!(
            fs::read_to_string(d.join("settings.json.bad")).unwrap(),
            "first {"
        );
        assert_eq!(
            fs::read_to_string(d.join("settings.json.bad.1")).unwrap(),
            "second {"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn one_value_this_version_cannot_read_resets_only_that_value() {
        let d = dir("lenient");
        fs::create_dir_all(&d).unwrap();
        let path = d.join("settings.json");
        fs::write(
            &path,
            r#"{"delay_secs": 5, "format": "Avif", "include_cursor": true}"#,
        )
        .unwrap();
        let l = Settings::load(&path);
        assert_eq!(l.settings.delay_secs, 5, "the readable values are kept");
        assert!(l.settings.include_cursor);
        assert_eq!(
            l.settings.format,
            ImageFormat::Png,
            "the unreadable one is reset"
        );
        let notice = l.notice.unwrap();
        assert!(notice.contains("format"), "{notice}");
        assert!(
            path.exists() && !d.join("settings.json.bad").exists(),
            "nothing is set aside"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_file_from_a_newer_version_is_kept_before_this_version_can_overwrite_it() {
        let d = dir("newer");
        fs::create_dir_all(&d).unwrap();
        let path = d.join("settings.json");
        let text = format!(
            r#"{{"version": {}, "delay_secs": 3, "a_future_option": [1, 2]}}"#,
            CURRENT_VERSION + 1
        );
        fs::write(&path, &text).unwrap();
        let l = Settings::load(&path);
        assert_eq!(l.settings.delay_secs, 3);
        let kept = d.join(format!("settings.json.v{}", CURRENT_VERSION + 1));
        assert_eq!(
            fs::read_to_string(&kept).unwrap(),
            text,
            "the original survives"
        );
        assert!(l.notice.unwrap().contains("newer version"));
        l.settings.save(&path).unwrap(); // dropping the unknown option from settings.json...
        assert_eq!(
            fs::read_to_string(&kept).unwrap(),
            text,
            "...does not touch the kept copy"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_huge_settings_file_is_set_aside_unread() {
        let d = dir("huge");
        fs::create_dir_all(&d).unwrap();
        let path = d.join("settings.json");
        fs::File::create(&path)
            .unwrap()
            .set_len(MAX_SETTINGS_BYTES + 1)
            .unwrap();
        let l = Settings::load(&path);
        assert_eq!(l.settings, Settings::default());
        assert!(!path.exists() && d.join("settings.json.bad").exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn invalid_shortcuts_are_reset_with_a_notice() {
        let d = dir("badshortcut");
        fs::create_dir_all(&d).unwrap();
        let path = d.join("settings.json");
        fs::write(
            &path,
            r#"{"shortcuts": {"capture_region": "R", "capture_window": "Ctrl+Banana"}}"#,
        )
        .unwrap();
        let l = Settings::load(&path);
        assert_eq!(l.settings.shortcuts, ShortcutSettings::default());
        let n = l.notice.unwrap();
        assert!(
            n.contains("Capture region") && n.contains("Capture window"),
            "{n}"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn duplicate_shortcuts_are_reset() {
        let mut s = Settings::default();
        s.shortcuts.capture_region = s.shortcuts.capture_full_screen.clone();
        let notes = s.sanitize();
        assert_eq!(notes.len(), 1);
        assert_eq!(s.shortcuts, ShortcutSettings::default());
    }

    #[test]
    fn newer_version_loads_with_a_notice_and_absurd_delay_is_clamped() {
        let d = dir("newer");
        fs::create_dir_all(&d).unwrap();
        let path = d.join("settings.json");
        fs::write(&path, r#"{"version": 99, "delay_secs": 100000}"#).unwrap();
        let l = Settings::load(&path);
        assert_eq!(l.settings.delay_secs, 60);
        assert_eq!(l.settings.version, CURRENT_VERSION);
        assert!(l.notice.unwrap().contains("newer version"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn failed_save_reports_the_path_and_leaves_no_temp_file() {
        let d = dir("failsave");
        fs::create_dir_all(&d).unwrap();
        // A directory where the file should be makes the final rename fail.
        let path = d.join("settings.json");
        fs::create_dir_all(path.join("blocker")).unwrap();
        let err = Settings::default().save(&path).unwrap_err();
        assert!(err.to_string().contains("settings.json"), "{err}");
        let leftovers: Vec<_> = fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn recent_lists_put_the_newest_first_without_duplicates_and_stay_short() {
        let mut list = Vec::new();
        for i in 0..15 {
            remember_recent(&mut list, &format!("/guides/g{i}"));
        }
        assert_eq!(list.len(), MAX_RECENT);
        assert_eq!(list[0], "/guides/g14");
        remember_recent(&mut list, "/guides/g10");
        assert_eq!(list[0], "/guides/g10", "reopening moves it to the front");
        assert_eq!(list.iter().filter(|p| *p == "/guides/g10").count(), 1);
        remember_recent(&mut list, "   ");
        assert_eq!(list.len(), MAX_RECENT, "blank paths are ignored");
    }

    #[test]
    fn new_settings_round_trip_and_old_files_without_them_still_load() {
        let d = dir("phase8");
        let path = d.join("settings.json");
        let s = Settings {
            language: "de".into(),
            theme: ThemeChoice::Light,
            recent_projects: vec!["/a.proj".into(), "/b.proj".into()],
            recent_edits: vec!["/c.edit".into()],
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).settings, s);
        fs::write(&path, r#"{"delay_secs": 3}"#).unwrap();
        let old = Settings::load(&path);
        assert_eq!(
            (old.settings.theme, old.settings.language.as_str()),
            (ThemeChoice::System, "")
        );
        assert_eq!(old.notice, None);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn messy_recent_lists_are_cleaned_up() {
        let mut s = Settings {
            recent_projects: vec!["/a".into(), "".into(), "/a".into(), "/b".into()],
            ..Settings::default()
        };
        s.sanitize();
        assert_eq!(s.recent_projects, ["/a", "/b"]);
    }

    #[test]
    fn click_capture_settings_round_trip_default_off_and_are_clamped() {
        let d = dir("clicks");
        let path = d.join("settings.json");
        let defaults = Settings::default();
        assert!(!defaults.click_marker);
        assert_eq!(defaults.click_max, 0);
        let mut s = Settings::default();
        s.active_mut().click_capture_options = ClickCaptureOptions {
            marker: true,
            max: 12,
        };
        s.sanitize(); // mirrors the active profile into the older top-level fields
        assert!(s.click_marker);
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).settings, s);
        fs::write(&path, r#"{"click_max": 999999}"#).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.settings.click_max, MAX_CLICK_SHOTS);
        assert!(loaded.notice.is_some());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn files_from_before_profiles_get_one_seeded_from_the_click_options() {
        let d = dir("seed-profile");
        fs::create_dir_all(&d).unwrap();
        let path = d.join("settings.json");
        fs::write(&path, r#"{"click_marker": true, "click_max": 7}"#).unwrap();
        let s = Settings::load(&path).settings;
        assert_eq!(s.profiles.len(), 1);
        assert_eq!(s.active().name, DEFAULT_PROFILE);
        assert_eq!(
            s.active().click_capture_options,
            ClickCaptureOptions {
                marker: true,
                max: 7
            }
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_selected_profile_and_its_options_survive_a_save_and_load() {
        let d = dir("profile-roundtrip");
        let path = d.join("settings.json");
        let mut s = Settings::default();
        s.add_profile("Tutorials").unwrap();
        s.active_mut().click_capture_options.max = 12;
        s.active_mut().editor_options.color = Rgba([1, 2, 3, 255]);
        s.sanitize();
        s.save(&path).unwrap();
        let back = Settings::load(&path).settings;
        assert_eq!(back.active_profile.as_deref(), Some("Tutorials"));
        assert_eq!(back.active().click_capture_options.max, 12);
        assert_eq!(back.active().editor_options.color, Rgba([1, 2, 3, 255]));
        assert_eq!(back, s);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn messy_profiles_are_cleaned_up_and_an_unknown_active_profile_falls_back() {
        let mut s = Settings {
            active_profile: Some("gone".into()),
            profiles: vec![
                Profile {
                    name: "  A ".into(),
                    ..Profile::default()
                },
                Profile {
                    name: "a".into(),
                    ..Profile::default()
                },
                Profile {
                    name: "   ".into(),
                    ..Profile::default()
                },
                Profile {
                    name: "B".into(),
                    click_capture_options: ClickCaptureOptions {
                        marker: true,
                        max: 99_999,
                    },
                    editor_options: EditorOptions {
                        width: f32::NAN,
                        text_size: 1e9,
                        ..EditorOptions::default()
                    },
                },
            ],
            ..Settings::default()
        };
        let notes = s.sanitize();
        let names: Vec<_> = s.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["A", "B"]);
        assert_eq!(s.active_profile.as_deref(), Some("A"));
        assert!(notes.iter().any(|n| n.contains("profile")), "{notes:?}");
        let b = &s.profiles[1];
        assert_eq!(b.click_capture_options.max, MAX_CLICK_SHOTS);
        assert_eq!(b.editor_options.width, 1.0);
        assert_eq!(b.editor_options.text_size, 400.0);
    }

    #[test]
    fn profiles_can_be_added_renamed_selected_and_deleted_but_never_emptied() {
        let mut s = Settings::default();
        assert_eq!(s.add_profile("  "), Err(ProfileError::EmptyName));
        assert_eq!(s.add_profile("default"), Err(ProfileError::Duplicate));
        s.add_profile("Docs").unwrap();
        assert_eq!(s.active_profile.as_deref(), Some("Docs"));
        s.rename_profile("Docs", "Manuals").unwrap();
        assert_eq!(s.active_profile.as_deref(), Some("Manuals"));
        assert_eq!(s.rename_profile("nope", "X"), Err(ProfileError::Missing));
        assert_eq!(
            s.rename_profile("Manuals", "Default"),
            Err(ProfileError::Duplicate)
        );
        s.select_profile("Default").unwrap();
        assert_eq!(s.select_profile("nope"), Err(ProfileError::Missing));
        s.select_profile("Manuals").unwrap();
        s.delete_profile("Manuals").unwrap();
        assert_eq!(s.active_profile.as_deref(), Some("Default"));
        assert_eq!(s.delete_profile("Default"), Err(ProfileError::LastProfile));
        for i in 1..MAX_PROFILES {
            s.add_profile(&format!("p{i}")).unwrap();
        }
        assert_eq!(s.add_profile("one more"), Err(ProfileError::TooMany));
    }

    #[test]
    fn a_new_profile_starts_as_a_copy_of_the_selected_one() {
        let mut s = Settings::default();
        s.active_mut().click_capture_options.max = 5;
        s.add_profile("Copy").unwrap();
        assert_eq!(s.active().click_capture_options.max, 5);
    }
}
