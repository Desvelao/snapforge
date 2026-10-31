//! A saved edit: the untouched original screenshot plus its editable annotations.
//!
//! ```text
//! shot.<edit extension>/
//!   original.png        written once by `create`, never rewritten
//!   annotations.json    versioned, replaced atomically on every save
//! ```
//!
//! The folder uses only relative names, so it can be moved or copied anywhere.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use guide_core::Document;
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};

use crate::{MAX_IMAGE_PIXELS, MAX_MANIFEST_BYTES};

pub const ORIGINAL_FILE: &str = "original.png";
pub const ANNOTATIONS_FILE: &str = "annotations.json";

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("{0} already contains an edit; refusing to overwrite its original screenshot")]
    AlreadyExists(PathBuf),
    #[error("the original screenshot is missing: {0}")]
    MissingOriginal(PathBuf),
    #[error("the annotations file is missing: {0}")]
    MissingAnnotations(PathBuf),
    #[error("cannot read the original screenshot {path}: {reason}")]
    BadOriginal { path: PathBuf, reason: String },
    #[error("the annotations file {path} is damaged: {reason}")]
    Corrupt { path: PathBuf, reason: String },
    #[error("the annotations in {path} cannot be used with this screenshot: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error(
        "{path} is too large ({width}x{height} pixels; the limit is {MAX_IMAGE_PIXELS} pixels)"
    )]
    TooLarge {
        path: PathBuf,
        width: u32,
        height: u32,
    },
    #[error("{path} is not a plain file inside the edit folder ({reason})")]
    Unsafe { path: PathBuf, reason: String },
    #[error("invalid image data: {0}")]
    InvalidImage(String),
    #[error("cannot access {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> BundleError {
    let path = path.to_path_buf();
    move |source| BundleError::Io { path, source }
}

/// The original image as stored: straight RGBA8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug)]
pub struct EditBundle {
    dir: PathBuf,
}

/// Write `bytes` to `path` so readers never see a partial file.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), BundleError> {
    crate::atomic::write_atomic(path, bytes).map_err(io_err(path))
}

/// The file must be a regular file in the edit folder itself: a link could point at (and so read
/// or reveal) a file anywhere else. `Ok(false)` when it does not exist.
fn plain_file(path: &Path) -> Result<bool, BundleError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => Ok(true),
        Ok(m) => Err(BundleError::Unsafe {
            path: path.to_path_buf(),
            reason: if m.file_type().is_symlink() {
                "it is a link".into()
            } else {
                "it is not a file".into()
            },
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_err(path)(e)),
    }
}

impl EditBundle {
    /// Starts a new edit in `dir`: stores the original and an empty document.
    /// Fails, touching nothing, if `dir` already holds an original.
    pub fn create(dir: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<Self, BundleError> {
        if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
            return Err(BundleError::InvalidImage(format!(
                "{width}x{height} with {} bytes",
                rgba.len()
            )));
        }
        if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
            return Err(BundleError::TooLarge {
                path: dir.to_path_buf(),
                width,
                height,
            });
        }
        let original = dir.join(ORIGINAL_FILE);
        if plain_file(&original)? {
            return Err(BundleError::AlreadyExists(dir.to_path_buf()));
        }
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(rgba, width, height, ExtendedColorType::Rgba8)
            .map_err(|e| BundleError::InvalidImage(e.to_string()))?;
        let made_dir = !dir.exists();
        fs::create_dir_all(dir).map_err(io_err(dir))?;
        let bundle = Self {
            dir: dir.to_path_buf(),
        };
        // `create_new` fails if somebody made the original since the check above, so an existing
        // original is never replaced; a failure later on removes what was made here.
        let made = (|| -> Result<(), BundleError> {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&original)
                .map_err(|e| {
                    if e.kind() == io::ErrorKind::AlreadyExists {
                        BundleError::AlreadyExists(dir.to_path_buf())
                    } else {
                        io_err(&original)(e)
                    }
                })?;
            let written = f.write_all(&png).and_then(|()| f.sync_all());
            if let Err(e) = written {
                let _ = fs::remove_file(&original);
                return Err(io_err(&original)(e));
            }
            if let Err(e) = bundle.save_document(&Document::default()) {
                let _ = fs::remove_file(&original);
                return Err(e);
            }
            Ok(())
        })();
        if let Err(e) = made {
            if made_dir && !matches!(e, BundleError::AlreadyExists(_)) {
                let _ = fs::remove_dir(dir);
            }
            return Err(e);
        }
        Ok(bundle)
    }

    /// Opens an existing edit and loads both parts, checking that they belong together.
    pub fn open(dir: &Path) -> Result<(Self, OriginalImage, Document), BundleError> {
        let bundle = Self {
            dir: dir.to_path_buf(),
        };
        let original = bundle.load_original()?;
        let document = bundle.load_document((original.width, original.height))?;
        Ok((bundle, original, document))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn load_original(&self) -> Result<OriginalImage, BundleError> {
        let path = self.dir.join(ORIGINAL_FILE);
        if !plain_file(&path)? {
            return Err(BundleError::MissingOriginal(path));
        }
        let bad = |reason: String| BundleError::BadOriginal {
            path: path.clone(),
            reason,
        };
        let (w, h) = image::image_dimensions(&path).map_err(|e| bad(e.to_string()))?;
        if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
            return Err(BundleError::TooLarge {
                path,
                width: w,
                height: h,
            });
        }
        let img = image::open(&path)
            .map_err(|e| BundleError::BadOriginal {
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

    pub fn load_document(&self, image_size: (u32, u32)) -> Result<Document, BundleError> {
        let path = self.dir.join(ANNOTATIONS_FILE);
        if !plain_file(&path)? {
            return Err(BundleError::MissingAnnotations(path));
        }
        let len = fs::metadata(&path).map_err(io_err(&path))?.len();
        if len > MAX_MANIFEST_BYTES {
            return Err(BundleError::Corrupt {
                path,
                reason: format!(
                    "the file is {} MB; the limit is {} MB",
                    len / (1024 * 1024),
                    MAX_MANIFEST_BYTES / (1024 * 1024)
                ),
            });
        }
        let text = fs::read_to_string(&path).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                BundleError::MissingAnnotations(path.clone())
            } else {
                io_err(&path)(e)
            }
        })?;
        let doc: Document = serde_json::from_str(&text).map_err(|e| BundleError::Corrupt {
            path: path.clone(),
            reason: e.to_string(),
        })?;
        doc.validate(image_size)
            .map_err(|reason| BundleError::Invalid { path, reason })?;
        Ok(doc)
    }

    /// Replaces only the annotations file. The original cannot be changed through this type.
    pub fn save_document(&self, doc: &Document) -> Result<(), BundleError> {
        let mut json = serde_json::to_string_pretty(doc)
            .map_err(|e| BundleError::InvalidImage(format!("cannot encode annotations: {e}")))?;
        json.push('\n');
        atomic_write(&self.dir.join(ANNOTATIONS_FILE), json.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guide_core::{Editor, Kind, PixelRect, Point, RectF, Style};

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("snapforge-bundle-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn pixels(w: u32, h: u32) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                [
                    (i % 251) as u8,
                    (i % 13) as u8 * 10,
                    (i / 7 % 256) as u8,
                    255,
                ]
            })
            .collect()
    }

    fn edited() -> Document {
        let mut e = Editor::new(Document::default());
        e.add(
            Kind::Rectangle {
                rect: RectF {
                    x: 1.0,
                    y: 2.0,
                    w: 3.0,
                    h: 4.0,
                },
            },
            Style::default(),
        );
        e.add_marker(Point::new(5.0, 5.0), 10.0, Style::default());
        e.set_crop(Some(PixelRect {
            x: 1,
            y: 1,
            w: 8,
            h: 8,
        }));
        e.document().clone()
    }

    #[test]
    fn create_then_open_round_trips_pixels_and_annotations() {
        let d = dir("roundtrip");
        let px = pixels(12, 9);
        let b = EditBundle::create(&d, 12, 9, &px).unwrap();
        let doc = edited();
        b.save_document(&doc).unwrap();
        let (_, original, loaded) = EditBundle::open(&d).unwrap();
        assert_eq!((original.width, original.height), (12, 9));
        assert!(original.rgba == px, "original pixels changed");
        assert!(loaded.annotations == doc.annotations && loaded.crop == doc.crop);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn saving_annotations_never_touches_the_original_file() {
        let d = dir("immutable");
        let b = EditBundle::create(&d, 8, 8, &pixels(8, 8)).unwrap();
        let before = fs::read(d.join(ORIGINAL_FILE)).unwrap();
        let mtime = fs::metadata(d.join(ORIGINAL_FILE))
            .unwrap()
            .modified()
            .unwrap();
        for _ in 0..5 {
            b.save_document(&edited()).unwrap();
        }
        assert!(
            fs::read(d.join(ORIGINAL_FILE)).unwrap() == before,
            "original bytes changed"
        );
        assert_eq!(
            fs::metadata(d.join(ORIGINAL_FILE))
                .unwrap()
                .modified()
                .unwrap(),
            mtime
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn create_refuses_to_overwrite_an_existing_original() {
        let d = dir("exists");
        EditBundle::create(&d, 4, 4, &pixels(4, 4)).unwrap();
        let before = fs::read(d.join(ORIGINAL_FILE)).unwrap();
        let err = EditBundle::create(&d, 4, 4, &[7u8; 64]).unwrap_err();
        assert!(matches!(err, BundleError::AlreadyExists(_)), "{err}");
        assert!(fs::read(d.join(ORIGINAL_FILE)).unwrap() == before);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_leaves_no_temp_files() {
        let d = dir("tmp");
        let b = EditBundle::create(&d, 4, 4, &pixels(4, 4)).unwrap();
        b.save_document(&edited_small()).unwrap();
        let mut names: Vec<_> = fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![ANNOTATIONS_FILE.to_string(), ORIGINAL_FILE.to_string()]
        );
        fs::remove_dir_all(&d).unwrap();
    }

    fn edited_small() -> Document {
        let mut e = Editor::new(Document::default());
        e.add(
            Kind::Line {
                from: Point::new(0.0, 0.0),
                to: Point::new(3.0, 3.0),
            },
            Style::default(),
        );
        e.document().clone()
    }

    #[test]
    fn a_damaged_annotations_file_is_reported_and_left_alone() {
        let d = dir("corrupt");
        EditBundle::create(&d, 4, 4, &pixels(4, 4)).unwrap();
        fs::write(d.join(ANNOTATIONS_FILE), "{ not json").unwrap();
        let err = EditBundle::open(&d).unwrap_err();
        assert!(matches!(err, BundleError::Corrupt { .. }), "{err}");
        assert!(err.to_string().contains(ANNOTATIONS_FILE));
        assert_eq!(
            fs::read_to_string(d.join(ANNOTATIONS_FILE)).unwrap(),
            "{ not json"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn missing_parts_have_clear_errors() {
        let d = dir("missing");
        EditBundle::create(&d, 4, 4, &pixels(4, 4)).unwrap();
        fs::remove_file(d.join(ANNOTATIONS_FILE)).unwrap();
        assert!(matches!(
            EditBundle::open(&d).unwrap_err(),
            BundleError::MissingAnnotations(_)
        ));
        fs::remove_file(d.join(ORIGINAL_FILE)).unwrap();
        let err = EditBundle::open(&d).unwrap_err();
        assert!(matches!(err, BundleError::MissingOriginal(_)), "{err}");
        assert!(err.to_string().contains("original"));
        assert!(matches!(
            EditBundle::open(&d.join("nope")).unwrap_err(),
            BundleError::MissingOriginal(_)
        ));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn annotations_that_do_not_fit_the_image_are_rejected() {
        let d = dir("invalid");
        let b = EditBundle::create(&d, 4, 4, &pixels(4, 4)).unwrap();
        let mut doc = Document::default();
        doc.crop = Some(PixelRect {
            x: 0,
            y: 0,
            w: 50,
            h: 50,
        });
        b.save_document(&doc).unwrap();
        assert!(matches!(
            EditBundle::open(&d).unwrap_err(),
            BundleError::Invalid { .. }
        ));

        fs::write(
            d.join(ANNOTATIONS_FILE),
            r#"{"version": 99, "annotations": [], "crop": null}"#,
        )
        .unwrap();
        let err = EditBundle::open(&d).unwrap_err();
        assert!(err.to_string().contains("newer version"), "{err}");
        fs::remove_dir_all(&d).unwrap();
    }

    /// Keeps docs/annotations.md honest: this is the structure it documents.
    #[test]
    fn the_saved_file_has_the_documented_structure() {
        let d = dir("structure");
        let b = EditBundle::create(&d, 400, 300, &vec![0u8; 400 * 300 * 4]).unwrap();
        let mut e = Editor::new(Document::default());
        e.add(
            Kind::Rectangle {
                rect: RectF {
                    x: 10.0,
                    y: 10.0,
                    w: 200.0,
                    h: 80.0,
                },
            },
            Style::default(),
        );
        e.add(
            Kind::Obscure {
                rect: RectF {
                    x: 50.0,
                    y: 120.0,
                    w: 300.0,
                    h: 40.0,
                },
                mode: guide_core::ObscureMode::Pixelate,
                strength: 12,
            },
            Style::default(),
        );
        e.set_crop(Some(PixelRect {
            x: 0,
            y: 0,
            w: 400,
            h: 300,
        }));
        b.save_document(e.document()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(d.join(ANNOTATIONS_FILE)).unwrap()).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["next_id"], 3);
        assert_eq!(v["crop"], serde_json::json!({"x":0,"y":0,"w":400,"h":300}));
        let a = &v["annotations"][0];
        assert_eq!(a["id"], 1);
        assert_eq!(a["kind"]["type"], "Rectangle");
        assert_eq!(
            a["kind"]["rect"],
            serde_json::json!({"x":10.0,"y":10.0,"w":200.0,"h":80.0})
        );
        assert_eq!(
            a["style"],
            serde_json::json!({"color":[230,40,40,255],"width":4.0,"fill":null})
        );
        let o = &v["annotations"][1]["kind"];
        assert_eq!(
            (
                o["type"].as_str(),
                o["mode"].as_str(),
                o["strength"].as_u64()
            ),
            (Some("Obscure"), Some("Pixelate"), Some(12))
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_non_png_original_is_reported() {
        let d = dir("badpng");
        EditBundle::create(&d, 4, 4, &pixels(4, 4)).unwrap();
        fs::write(d.join(ORIGINAL_FILE), b"not a png").unwrap();
        assert!(matches!(
            EditBundle::open(&d).unwrap_err(),
            BundleError::BadOriginal { .. }
        ));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_folder_can_be_moved_and_still_opens() {
        let a = dir("move-a");
        let b = dir("move-b");
        let bundle = EditBundle::create(&a, 12, 12, &pixels(12, 12)).unwrap();
        bundle.save_document(&edited()).unwrap();
        fs::rename(&a, &b).unwrap();
        let (_, original, doc) = EditBundle::open(&b).unwrap();
        assert_eq!(original.width, 12);
        assert_eq!(doc.annotations.len(), 2);
        fs::remove_dir_all(&b).unwrap();
    }

    #[test]
    fn invalid_image_data_is_refused_before_anything_is_written() {
        let d = dir("badimage");
        assert!(matches!(
            EditBundle::create(&d, 4, 4, &[0u8; 10]),
            Err(BundleError::InvalidImage(_))
        ));
        assert!(matches!(
            EditBundle::create(&d, 0, 4, &[]),
            Err(BundleError::InvalidImage(_))
        ));
        assert!(!d.exists());
    }

    /// The bytes of a PNG that only has a header, claiming to be `w` x `h`.
    fn png_header_claiming(w: u32, h: u32) -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = !0u32;
            for &b in data {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        let mut ihdr = b"IHDR".to_vec();
        ihdr.extend(w.to_be_bytes());
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([8, 2, 0, 0, 0]); // 8 bit RGB
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        out.extend(13u32.to_be_bytes());
        out.extend(&ihdr);
        out.extend(crc32(&ihdr).to_be_bytes());
        // The decoder reads up to the start of the pixel data before it reports the size.
        out.extend(0u32.to_be_bytes());
        out.extend(b"IDAT");
        out.extend(crc32(b"IDAT").to_be_bytes());
        out
    }

    #[test]
    fn an_original_that_claims_a_huge_size_is_refused_before_it_is_decoded() {
        let d = dir("huge");
        EditBundle::create(&d, 4, 4, &pixels(4, 4)).unwrap();
        fs::write(d.join(ORIGINAL_FILE), png_header_claiming(30_000, 30_000)).unwrap();
        let err = EditBundle::open(&d).unwrap_err();
        assert!(matches!(err, BundleError::TooLarge { .. }), "{err}");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_oversized_annotations_file_is_refused_without_reading_it() {
        let d = dir("big-annotations");
        let b = EditBundle::create(&d, 4, 4, &pixels(4, 4)).unwrap();
        let f = fs::File::create(d.join(ANNOTATIONS_FILE)).unwrap();
        f.set_len(MAX_MANIFEST_BYTES + 1).unwrap(); // sparse: costs no disk
        let err = b.load_document((4, 4)).unwrap_err();
        assert!(matches!(err, BundleError::Corrupt { .. }), "{err}");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn links_in_place_of_the_two_files_are_refused() {
        let d = dir("links");
        let outside = dir("links-outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.json"), "{}").unwrap();
        let b = EditBundle::create(&d, 4, 4, &pixels(4, 4)).unwrap();
        fs::remove_file(d.join(ANNOTATIONS_FILE)).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.json"), d.join(ANNOTATIONS_FILE)).unwrap();
        let err = b.load_document((4, 4)).unwrap_err();
        assert!(matches!(err, BundleError::Unsafe { .. }), "{err}");

        fs::remove_file(d.join(ORIGINAL_FILE)).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.json"), d.join(ORIGINAL_FILE)).unwrap();
        let err = b.load_original().unwrap_err();
        assert!(matches!(err, BundleError::Unsafe { .. }), "{err}");
        // And create does not write through a link that stands where the original would go.
        let err = EditBundle::create(&d, 4, 4, &pixels(4, 4)).unwrap_err();
        assert!(matches!(err, BundleError::Unsafe { .. }), "{err}");
        assert_eq!(
            fs::read_to_string(outside.join("secret.json")).unwrap(),
            "{}"
        );
        fs::remove_dir_all(&d).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn a_failed_create_leaves_no_folder_behind() {
        let d = dir("failed-create");
        // A size that does not match the data fails before anything is made.
        assert!(EditBundle::create(&d, 4, 4, &[0u8; 3]).is_err());
        assert!(!d.exists());
        // The folder that was there before is kept, with what it already held.
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("notes.txt"), "mine").unwrap();
        EditBundle::create(&d, 4, 4, &pixels(4, 4)).unwrap();
        assert!(d.join("notes.txt").exists());
        fs::remove_dir_all(&d).unwrap();
    }
}
