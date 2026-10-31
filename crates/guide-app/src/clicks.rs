//! `snapforge clicks`: save a screenshot of every left click, without any window.

use std::{path::PathBuf, sync::mpsc, thread, time::Duration};

use guide_capture::{
    detect_backend, run_click_session, CancelToken, CaptureMode, CaptureRequest, Support,
};
use guide_storage::{save_rgba, timestamped_stem, Settings};

pub const USAGE: &str = "usage: snapforge clicks [--count N] [--path FOLDER]";

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    /// Stop after this many shots; `None` runs until interrupted (Ctrl+C).
    pub count: Option<usize>,
    pub path: Option<PathBuf>,
}

pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        count: None,
        path: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--count" => {
                let n: usize = value("--count")?
                    .parse()
                    .map_err(|_| "--count must be a whole number".to_string())?;
                if n == 0 {
                    return Err("--count must be at least 1".into());
                }
                options.count = Some(n);
            }
            "--path" => options.path = Some(PathBuf::from(value("--path")?)),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(options)
}

/// Runs until `--count` shots were saved or the process is interrupted. The exit code is 0 on success.
pub fn run(args: &[String]) -> i32 {
    let options = match parse_args(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("snapforge clicks: {e}\n{USAGE}");
            return 64;
        }
    };
    let settings = Settings::default_path()
        .ok()
        .map(|p| Settings::load(&p).settings)
        .unwrap_or_default();
    let dir = options
        .path
        .unwrap_or_else(|| PathBuf::from(&settings.out_dir));
    let backend = detect_backend();
    let caps = backend.capabilities();
    if let Support::No(why) = &caps.full_screen {
        eprintln!("snapforge clicks: capture is not available: {why}");
        return 1;
    }
    let pointer = guide_hotkeys::detect_pointer(|| {});
    if let Err(why) = pointer.availability() {
        eprintln!("snapforge clicks: click capture is not available: {why}");
        return 1;
    }
    let request = CaptureRequest {
        include_cursor: settings.include_cursor && caps.cursor.is_yes(),
        mode: CaptureMode::FullScreen,
    };
    let cancel = CancelToken::default();
    let (tx, rx) = mpsc::channel();
    let forwarding = cancel.clone();
    thread::spawn(move || {
        while !forwarding.is_cancelled() {
            match pointer.try_recv() {
                Some(c) if c.button == 1 => {
                    if tx.send((c.x, c.y)).is_err() {
                        break;
                    }
                }
                Some(_) => {}
                None => thread::sleep(Duration::from_millis(10)),
            }
        }
    });
    eprintln!(
        "snapforge clicks: saving a screenshot of every left click to {} (Ctrl+C stops)",
        dir.display()
    );
    let mut failed = false;
    let result = run_click_session(
        &*backend,
        &request,
        &rx,
        &cancel,
        options.count,
        Duration::from_millis(150),
        |shot, _| match save_rgba(
            &dir,
            &timestamped_stem(),
            settings.format,
            shot.width,
            shot.height,
            &shot.rgba,
        ) {
            Ok(path) => println!("{}", path.display()),
            Err(e) => {
                eprintln!("snapforge clicks: could not save the screenshot: {e}");
                failed = true;
                cancel.cancel();
            }
        },
    );
    cancel.cancel();
    if let Err(e) = result {
        eprintln!("snapforge clicks: capture failed: {e}");
        return 1;
    }
    i32::from(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn count_and_path_are_read_and_bad_input_is_explained() {
        assert_eq!(
            parse_args(&args(&["--count", "3", "--path", "/tmp/x"])),
            Ok(Options {
                count: Some(3),
                path: Some("/tmp/x".into())
            })
        );
        assert_eq!(
            parse_args(&[]),
            Ok(Options {
                count: None,
                path: None
            })
        );
        assert!(parse_args(&args(&["--count", "0"])).is_err());
        assert!(parse_args(&args(&["--count", "many"])).is_err());
        assert!(parse_args(&args(&["--path"])).is_err());
        assert!(parse_args(&args(&["--nope"])).is_err());
    }
}
