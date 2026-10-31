//! System file and folder dialogs (through the desktop portal), run off the UI thread.

use eframe::egui;

use crate::i18n::tr;

use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// A dialog that closes sooner than a person could answer it did not open at all
/// (no portal on the session bus); the portal backend reports that like a cancel.
const UNAVAILABLE_BELOW: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    Chosen(PathBuf),
    Cancelled,
    /// There is no dialog on this system.
    Unavailable,
}

#[derive(Clone, Copy)]
pub enum Kind {
    Folder,
    OpenImage,
}

fn outcome(chosen: Option<PathBuf>, started: Instant) -> Pick {
    match chosen {
        Some(p) => Pick::Chosen(p),
        None if started.elapsed() < UNAVAILABLE_BELOW => Pick::Unavailable,
        None => Pick::Cancelled,
    }
}

fn start_dir(dir: &Path) -> &Path {
    if dir.is_dir() {
        dir
    } else {
        dir.parent()
            .filter(|p| p.is_dir())
            .unwrap_or(Path::new("/"))
    }
}

/// Blocks until the user has answered. Call it from a worker thread only.
pub fn save_file(dir: &Path, name: &str, title: &str) -> Pick {
    let started = Instant::now();
    let dialog = rfd::AsyncFileDialog::new()
        .set_title(title)
        .set_directory(start_dir(dir))
        .set_file_name(name)
        .add_filter("PNG", &["png"])
        .add_filter("JPEG", &["jpg", "jpeg"])
        .add_filter("WebP", &["webp"]);
    let chosen = pollster::block_on(dialog.save_file()).map(|h| h.path().to_path_buf());
    outcome(chosen, started)
}

/// Blocks until the user has answered. Call it from a worker thread only.
pub fn pick(kind: Kind, dir: &Path, title: &str) -> Pick {
    let started = Instant::now();
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title(title)
        .set_directory(start_dir(dir));
    if let Kind::OpenImage = kind {
        dialog = dialog.add_filter("Images", &["png", "jpg", "jpeg", "webp"]);
    }
    let chosen = match kind {
        Kind::Folder => pollster::block_on(dialog.pick_folder()),
        Kind::OpenImage => pollster::block_on(dialog.pick_file()),
    }
    .map(|h| h.path().to_path_buf());
    outcome(chosen, started)
}

/// A dialog opened from a button, answered on a later frame.
/// `T` says which field the answer belongs to.
pub struct Pending<T> {
    open: Option<(T, mpsc::Receiver<Pick>)>,
}

impl<T> Default for Pending<T> {
    fn default() -> Self {
        Self { open: None }
    }
}

impl<T> Pending<T> {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Opens a dialog unless one is already open.
    pub fn start(&mut self, ctx: &egui::Context, target: T, kind: Kind, from: &str, title: &str) {
        if self.open.is_some() {
            return;
        }
        let (dir, title, ctx) = (expand_home(from), title.to_string(), ctx.clone());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(pick(kind, &dir, &title));
            ctx.request_repaint();
        });
        self.open = Some((target, rx));
    }

    /// A "Browse..." button that opens a dialog starting at `from`.
    pub fn browse_button(
        &mut self,
        ui: &mut egui::Ui,
        target: T,
        kind: Kind,
        from: &str,
        title: &str,
    ) {
        let idle = !self.is_open();
        if ui
            .add_enabled(idle, egui::Button::new(tr("Browse...")))
            .clicked()
        {
            self.start(ui.ctx(), target, kind, from, title);
        }
    }

    /// The path chosen since the last call, with the field it was meant for.
    pub fn take_chosen(&mut self) -> Option<(T, PathBuf)> {
        let answer = self.open.as_ref()?.1.try_recv();
        match answer {
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.open = None;
                None
            }
            Ok(pick) => {
                let (target, _) = self.open.take()?;
                match pick {
                    Pick::Chosen(p) => Some((target, p)),
                    Pick::Cancelled | Pick::Unavailable => None,
                }
            }
        }
    }
}

fn expand_home(path: &str) -> PathBuf {
    let path = path.trim();
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialog_that_answers_at_once_with_nothing_is_unavailable_not_cancelled() {
        assert_eq!(outcome(None, Instant::now()), Pick::Unavailable);
        let long_ago = Instant::now() - Duration::from_secs(5);
        assert_eq!(outcome(None, long_ago), Pick::Cancelled);
        assert_eq!(
            outcome(Some("/a/b.png".into()), Instant::now()),
            Pick::Chosen("/a/b.png".into())
        );
    }

    #[test]
    fn the_dialog_starts_in_the_nearest_folder_that_exists() {
        assert_eq!(start_dir(Path::new("/tmp")), Path::new("/tmp"));
        assert_eq!(
            start_dir(Path::new("/tmp/does-not-exist-xyz/sub")),
            Path::new("/")
        );
        assert_eq!(
            start_dir(Path::new("/tmp/does-not-exist-xyz")),
            Path::new("/tmp")
        );
    }

    #[test]
    fn a_leading_tilde_means_the_home_folder() {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        if let Some(home) = home {
            assert_eq!(expand_home("~/Pictures"), home.join("Pictures"));
        }
        assert_eq!(expand_home(" /x/y "), PathBuf::from("/x/y"));
    }
}
