use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use image::{codecs::jpeg::JpegEncoder, ExtendedColorType, ImageEncoder};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
    /// Lossless only (the `image` crate has no lossy WebP encoder).
    WebP,
}

impl ImageFormat {
    pub const ALL: [ImageFormat; 3] = [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP];

    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::WebP => "webp",
        }
    }

    /// The format a file name's extension asks for (case-insensitive).
    pub fn from_extension(ext: &str) -> Option<ImageFormat> {
        let ext = ext.to_ascii_lowercase();
        ImageFormat::ALL
            .into_iter()
            .find(|f| f.extension() == ext || (*f == ImageFormat::Jpeg && ext == "jpeg"))
    }

    pub fn label(self) -> &'static str {
        match self {
            ImageFormat::Png => "PNG",
            ImageFormat::Jpeg => "JPEG",
            ImageFormat::WebP => "WebP (lossless)",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("invalid image: {0}")]
    InvalidImage(String),
    #[error("cannot write {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("encoding failed: {0}")]
    Encode(String),
}

/// `YYYY-MM-DD_HH-MM-SS` in UTC, suitable for a file stem.
pub fn timestamped_stem() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    stem_from_unix(secs)
}

fn stem_from_unix(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}-{:02}-{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Encodes `rgba` and writes it to `dir/<stem>.<ext>` without overwriting:
/// an existing name gets a `-2`, `-3`, ... suffix. The file appears
/// atomically (temp file + rename). Returns the final path.
pub fn save_rgba(
    dir: &Path,
    stem: &str,
    format: ImageFormat,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<PathBuf, SaveError> {
    write_rgba(
        dir,
        stem,
        format,
        width,
        height,
        rgba,
        ExistingFile::KeepBoth,
    )
}

/// Writes to exactly `path` (a name the user picked in a save dialog), replacing any
/// file there. The extension picks the format; one that names no format keeps the full
/// file name and uses `fallback`. Returns the final path.
pub fn save_rgba_at(
    path: &Path,
    fallback: ImageFormat,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<PathBuf, SaveError> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let (stem, format) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => match ImageFormat::from_extension(ext) {
            Some(f) => (stem.to_string(), f),
            None => (name.clone(), fallback),
        },
        _ => (name.clone(), fallback),
    };
    write_rgba(
        dir,
        &stem,
        format,
        width,
        height,
        rgba,
        ExistingFile::Replace,
    )
}

/// What to do when `dir/<stem>.<ext>` already exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExistingFile {
    /// Leave it alone and write to `<stem>-2.<ext>` (then `-3`, ...).
    #[default]
    KeepBoth,
    /// Replace it. The new file is written completely first and swapped in with a
    /// rename, so the old one is only ever replaced by a whole new one.
    Replace,
}

/// Like [`save_rgba`], with a choice of what happens to an existing file.
pub fn write_rgba(
    dir: &Path,
    stem: &str,
    format: ImageFormat,
    width: u32,
    height: u32,
    rgba: &[u8],
    existing: ExistingFile,
) -> Result<PathBuf, SaveError> {
    if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
        return Err(SaveError::InvalidImage(format!(
            "{width}x{height} with {} bytes",
            rgba.len()
        )));
    }
    let io_err = |path: &Path| {
        let path = path.to_path_buf();
        move |source| SaveError::Io { path, source }
    };
    fs::create_dir_all(dir).map_err(io_err(dir))?;

    let mut encoded = Vec::new();
    encode(&mut encoded, format, width, height, rgba)?;

    let tmp = crate::atomic::write_temp(dir, stem, |f| f.write_all(&encoded)).map_err(|e| {
        SaveError::Io {
            path: dir.to_path_buf(),
            source: e,
        }
    })?;
    if existing == ExistingFile::Replace {
        let target = dir.join(format!("{stem}.{}", format.extension()));
        return match fs::rename(&tmp, &target) {
            Ok(()) => {
                crate::atomic::sync_dir(dir);
                Ok(target)
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                Err(io_err(&target)(e))
            }
        };
    }
    for n in 1u32.. {
        let name = if n == 1 {
            format!("{stem}.{}", format.extension())
        } else {
            format!("{stem}-{n}.{}", format.extension())
        };
        let target = dir.join(name);
        match link_no_clobber(&tmp, &target, |from, to| fs::hard_link(from, to)) {
            Ok(()) => {
                let _ = fs::remove_file(&tmp);
                crate::atomic::sync_dir(dir);
                return Ok(target);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(io_err(&target)(e));
            }
        }
    }
    unreachable!("unbounded counter loop")
}

/// Gives `tmp`'s content the name `target` unless `target` exists (then `AlreadyExists`).
/// A hard link does it atomically; file systems without links (FAT, some network and FUSE
/// mounts) fall back to creating the target exclusively and copying into it.
fn link_no_clobber(
    tmp: &Path,
    target: &Path,
    link: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    match link(tmp, target) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(_) => {
            let mut to = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            let copied = fs::File::open(tmp)
                .and_then(|mut from| io::copy(&mut from, &mut to))
                .and_then(|_| to.sync_all());
            if let Err(e) = copied {
                let _ = fs::remove_file(target);
                return Err(e);
            }
            Ok(())
        }
        Ok(()) => Ok(()),
    }
}

fn encode(
    out: &mut Vec<u8>,
    format: ImageFormat,
    w: u32,
    h: u32,
    rgba: &[u8],
) -> Result<(), SaveError> {
    let enc = |e: image::ImageError| SaveError::Encode(e.to_string());
    match format {
        ImageFormat::Png => image::codecs::png::PngEncoder::new(out)
            .write_image(rgba, w, h, ExtendedColorType::Rgba8)
            .map_err(enc),
        ImageFormat::WebP => image::codecs::webp::WebPEncoder::new_lossless(out)
            .write_image(rgba, w, h, ExtendedColorType::Rgba8)
            .map_err(enc),
        ImageFormat::Jpeg => {
            let rgb: Vec<u8> = rgba
                .chunks_exact(4)
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            JpegEncoder::new_with_quality(out, 90)
                .write_image(&rgb, w, h, ExtendedColorType::Rgb8)
                .map_err(enc)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("snapforge-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn px() -> Vec<u8> {
        [10, 20, 30, 255].repeat(4)
    }

    #[test]
    fn the_extension_of_a_file_name_picks_the_image_format() {
        assert_eq!(ImageFormat::from_extension("PNG"), Some(ImageFormat::Png));
        assert_eq!(ImageFormat::from_extension("jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension("jpg"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension("webp"), Some(ImageFormat::WebP));
        assert_eq!(ImageFormat::from_extension("txt"), None);
    }

    #[test]
    fn saving_to_a_chosen_path_writes_exactly_there_and_replaces_a_file() {
        let dir = tmpdir("save-at");
        let path = dir.join("my shot.png");
        let first = save_rgba_at(&path, ImageFormat::Jpeg, 2, 2, &px()).unwrap();
        assert_eq!(first, path);
        fs::write(&path, b"old").unwrap();
        let second = save_rgba_at(&path, ImageFormat::Jpeg, 2, 2, &px()).unwrap();
        assert_eq!(second, path);
        assert!(fs::read(&path).unwrap().starts_with(b"\x89PNG"));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_chosen_name_without_an_image_extension_gets_the_fallback_format() {
        let dir = tmpdir("save-at-ext");
        let saved = save_rgba_at(&dir.join("shot.v2"), ImageFormat::Png, 2, 2, &px()).unwrap();
        assert_eq!(saved, dir.join("shot.v2.png"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stem_formats_known_dates() {
        assert_eq!(stem_from_unix(0), "1970-01-01_00-00-00");
        assert_eq!(stem_from_unix(1_782_667_800), "2026-06-28_17-30-00");
        assert_eq!(stem_from_unix(951_827_696), "2000-02-29_12-34-56");
    }

    #[test]
    fn saves_all_formats_and_roundtrips_png() {
        let dir = tmpdir("fmt");
        for f in ImageFormat::ALL {
            let p = save_rgba(&dir, "shot", f, 2, 2, &px()).unwrap();
            assert!(p.extension().unwrap() == f.extension());
            assert!(fs::metadata(&p).unwrap().len() > 0);
        }
        let back = image::open(dir.join("shot.png")).unwrap().to_rgba8();
        assert_eq!(back.into_raw(), px());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn never_overwrites_and_leaves_no_temp_files() {
        let dir = tmpdir("dup");
        let a = save_rgba(&dir, "s", ImageFormat::Png, 2, 2, &px()).unwrap();
        let b = save_rgba(&dir, "s", ImageFormat::Png, 2, 2, &px()).unwrap();
        assert_eq!(a.file_name().unwrap(), "s.png");
        assert_eq!(b.file_name().unwrap(), "s-2.png");
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replace_swaps_in_a_whole_new_file_and_keep_both_does_not() {
        let dir = tmpdir("replace");
        let red = [255u8, 0, 0, 255].repeat(4);
        let blue = [0u8, 0, 255, 255].repeat(4);
        let a = write_rgba(
            &dir,
            "x",
            ImageFormat::Png,
            2,
            2,
            &red,
            ExistingFile::Replace,
        )
        .unwrap();
        let b = write_rgba(
            &dir,
            "x",
            ImageFormat::Png,
            2,
            2,
            &blue,
            ExistingFile::Replace,
        )
        .unwrap();
        assert_eq!(a, b, "replacing keeps the same name");
        assert_eq!(image::open(&b).unwrap().to_rgba8().into_raw(), blue);
        let c = write_rgba(
            &dir,
            "x",
            ImageFormat::Png,
            2,
            2,
            &red,
            ExistingFile::KeepBoth,
        )
        .unwrap();
        assert_eq!(c.file_name().unwrap(), "x-2.png");
        assert_eq!(
            image::open(&b).unwrap().to_rgba8().into_raw(),
            blue,
            "the existing file was left alone"
        );
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "no temp files left: {names:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_replace_leaves_the_old_file_and_no_temp_file() {
        let dir = tmpdir("replace-fail");
        fs::create_dir_all(dir.join("x.png")).unwrap(); // a folder where the file should go
        let err = write_rgba(
            &dir,
            "x",
            ImageFormat::Png,
            2,
            2,
            &[0u8; 16],
            ExistingFile::Replace,
        )
        .unwrap_err();
        assert!(err.to_string().contains("x.png"), "{err}");
        assert!(dir.join("x.png").is_dir());
        assert!(fs::read_dir(&dir).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_hard_links_the_file_is_copied_and_still_never_clobbers() {
        let d = tmpdir("nolinks");
        fs::create_dir_all(&d).unwrap();
        let (tmp, target) = (d.join("a.tmp"), d.join("a.png"));
        fs::write(&tmp, b"new").unwrap();
        let unsupported = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::Unsupported));
        link_no_clobber(&tmp, &target, unsupported).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        // The target exists now: the fallback must refuse, not overwrite.
        fs::write(&target, b"mine").unwrap();
        let err = link_no_clobber(&tmp, &target, unsupported).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&target).unwrap(), b"mine");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_name_taken_during_the_link_is_reported_as_taken() {
        let d = tmpdir("taken");
        fs::create_dir_all(&d).unwrap();
        let (tmp, target) = (d.join("a.tmp"), d.join("a.png"));
        fs::write(&tmp, b"new").unwrap();
        let taken = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::AlreadyExists));
        let err = link_no_clobber(&tmp, &target, taken).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(!target.exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn odd_chosen_names_keep_their_full_name_and_get_the_fallback_extension() {
        let dir = tmpdir("odd-names");
        fs::create_dir_all(&dir).unwrap();
        let name = |file: &str, fallback| {
            let p = save_rgba_at(&dir.join(file), fallback, 2, 2, &px()).unwrap();
            p.file_name().unwrap().to_string_lossy().into_owned()
        };
        assert_eq!(name("foo.tar", ImageFormat::Png), "foo.tar.png");
        assert_eq!(name(".hidden", ImageFormat::WebP), ".hidden.webp");
        assert_eq!(name("noext", ImageFormat::Jpeg), "noext.jpg");
        assert_eq!(name("shout.PNG", ImageFormat::Jpeg), "shout.png");
        assert_eq!(name("a.b.jpeg", ImageFormat::Png), "a.b.jpg");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_temp_file_left_by_a_crash_does_not_disturb_either_mode() {
        let dir = tmpdir("stale-tmp");
        fs::create_dir_all(&dir).unwrap();
        // What an earlier run that died mid-save would leave beside the target.
        fs::write(dir.join(".shot.1234.tmp"), "half a file").unwrap();
        fs::write(dir.join(".shot.png.1234.tmp"), "half a file").unwrap();
        let first = save_rgba(&dir, "shot", ImageFormat::Png, 2, 2, &px()).unwrap();
        let second = save_rgba(&dir, "shot", ImageFormat::Png, 2, 2, &px()).unwrap();
        assert_eq!(first.file_name().unwrap(), "shot.png");
        assert_eq!(second.file_name().unwrap(), "shot-2.png");
        let replaced = write_rgba(
            &dir,
            "shot",
            ImageFormat::Png,
            2,
            2,
            &px(),
            ExistingFile::Replace,
        )
        .unwrap();
        assert_eq!(replaced, first);
        assert!(
            image::open(&replaced).is_ok(),
            "a whole image, not the stale bytes"
        );
        assert_eq!(
            fs::read_to_string(dir.join(".shot.1234.tmp")).unwrap(),
            "half a file",
            "other programs' files are left alone"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_bad_buffer() {
        let r = save_rgba(&tmpdir("bad"), "s", ImageFormat::Png, 2, 2, &[0; 3]);
        assert!(matches!(r, Err(SaveError::InvalidImage(_))));
    }
}
