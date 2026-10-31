//! A clipboard image that outlives the program that copied it.
//!
//! On X11 the clipboard belongs to the process that set it and disappears when that process
//! exits, unless a clipboard manager keeps a copy (many desktops, XFCE included, have none).
//! So the image is handed to a tiny background process (`snapforge --serve-clipboard <png>`)
//! that owns the clipboard until another program copies something, then exits.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use std::os::unix::process::CommandExt;

use guide_render::Image;
use guide_storage::{save_rgba, ImageFormat};

/// Puts `img` on the clipboard and returns at once; the clipboard stays filled after we exit.
pub fn copy_image_persistent(img: &Image) -> Result<(), String> {
    let dir = private_dir()?;
    let png = save_rgba(
        &dir,
        &format!("clip-{}", std::process::id()),
        ImageFormat::Png,
        img.width,
        img.height,
        &img.rgba,
    )
    .map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .arg("--serve-clipboard")
        .arg(&png)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own process group, so closing our terminal or window group does not take it down.
        .process_group(0)
        .spawn()
        .map_err(|e| {
            let _ = fs::remove_file(&png);
            e.to_string()
        })?;
    // Reap it when it ends if we are still running (the main window), so no zombie is left.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// A folder only this user can read, for the short-lived image file.
fn private_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("snapforge-clipboard-{}", users_id()));
    match fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => Ok(dir),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            check_private(&dir, own_uid()?)?;
            Ok(dir)
        }
        Err(e) => Err(format!("cannot create {}: {e}", dir.display())),
    }
}

/// An existing hand-over folder is only safe if it is a real folder (not a link) that we own and
/// nobody else can enter; in the shared temp folder another user could have made it first.
fn check_private(dir: &Path, uid: u32) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta =
        fs::symlink_metadata(dir).map_err(|e| format!("cannot inspect {}: {e}", dir.display()))?;
    if !meta.is_dir() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "{} exists but is not a private folder owned by you",
            dir.display()
        ));
    }
    Ok(())
}

/// The user id of this process, taken from the owner of `/proc/self`.
fn own_uid() -> Result<u32, String> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata("/proc/self")
        .map(|m| m.uid())
        .map_err(|e| format!("cannot find out who we are: {e}"))
}

/// Distinguishes users sharing one temp folder without needing libc.
fn users_id() -> String {
    own_uid().map_or_else(|_| "user".into(), |uid| uid.to_string())
}

/// The hidden `--serve-clipboard <png>` mode: owns the clipboard until something else is copied.
pub fn serve(png: &Path) -> Result<(), String> {
    use arboard::SetExtLinux;
    let decoded = image::open(png).map_err(|e| e.to_string());
    // The file is only a hand-over; remove it whether or not it could be read.
    let _ = fs::remove_file(png);
    let rgba = decoded?.into_rgba8();
    let (width, height) = rgba.dimensions();
    arboard::Clipboard::new()
        .map_err(|e| e.to_string())?
        .set()
        .wait()
        .image(arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: rgba.into_raw().into(),
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sg-clip-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn an_existing_private_folder_of_ours_is_accepted() {
        let dir = scratch("ok");
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        assert!(check_private(&dir, own_uid().unwrap()).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_owned_by_someone_else_is_refused() {
        let dir = scratch("other");
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        assert!(check_private(&dir, own_uid().unwrap() + 1).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_others_can_enter_is_refused() {
        let dir = scratch("open");
        fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(check_private(&dir, own_uid().unwrap()).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_symlink_in_place_of_the_folder_is_refused() {
        let target = scratch("target");
        fs::DirBuilder::new().mode(0o700).create(&target).unwrap();
        let link = scratch("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(check_private(&link, own_uid().unwrap()).is_err());
        fs::remove_file(&link).unwrap();
        fs::remove_dir_all(&target).unwrap();
    }
}
