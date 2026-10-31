//! The one way this crate writes a file so that readers never see a half-written one.
//!
//! The bytes go to a temp file beside the target (flushed to disk), the temp file is renamed over
//! the target, and the folder is flushed so the rename itself survives a power loss.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp file name in `dir` that no other save in this process (or another process) uses.
pub(crate) fn temp_path(dir: &Path, name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".{name}.{}.{n}.tmp", std::process::id()))
}

/// Flushes the folder entry so a rename or new file in it is durable. Best effort: some file
/// systems cannot open a folder for this, and the data itself is already safe.
pub(crate) fn sync_dir(dir: &Path) {
    if let Ok(f) = fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

/// Creates the temp file, runs `fill` on it and flushes it to disk. Removes it on any failure.
pub(crate) fn write_temp(
    dir: &Path,
    name: &str,
    fill: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let tmp = temp_path(dir, name);
    let result = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        fill(&mut f)?;
        f.sync_all()
    })();
    match result {
        Ok(()) => Ok(tmp),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Replaces `path` with `bytes` atomically.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let tmp = write_temp(dir, &name, |f| f.write_all(bytes))?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    sync_dir(dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sg-atomic-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn writing_replaces_the_file_and_leaves_no_temp_file() {
        let d = dir("replace");
        let f = d.join("a.json");
        write_atomic(&f, b"one").unwrap();
        write_atomic(&f, b"two").unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"two");
        let names: Vec<_> = fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["a.json"]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn temp_names_are_unique_within_one_process() {
        let d = Path::new("/x");
        assert_ne!(temp_path(d, "a"), temp_path(d, "a"));
    }

    #[test]
    fn many_threads_saving_the_same_file_never_corrupt_it_or_each_other() {
        let d = dir("threads");
        let f = d.join("shared.json");
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let f = f.clone();
                std::thread::spawn(move || {
                    for i in 0..25 {
                        let body = format!("{t}-{i}-").repeat(2_000);
                        write_atomic(&f, body.as_bytes()).expect("a concurrent save failed");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        // Whatever save won last, the file is exactly one whole body, never a mix.
        let text = fs::read_to_string(&f).unwrap();
        let unit = text.split('-').take(2).collect::<Vec<_>>().join("-") + "-";
        assert_eq!(text, unit.repeat(2_000));
        let names = fs::read_dir(&d).unwrap().count();
        assert_eq!(names, 1, "no temp files are left behind");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_failed_write_removes_its_temp_file() {
        let d = dir("fail");
        let err = write_temp(&d, "x", |_| Err(io::Error::other("disk full")));
        assert!(err.is_err());
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);
        fs::remove_dir_all(&d).unwrap();
    }
}
