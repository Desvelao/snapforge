//! `snapforge gui`: capture first, then a frozen-screen region selection and the editor, with no
//! main window. Meant to be bound to a desktop keyboard shortcut (like `flameshot gui`).
use std::{path::PathBuf, time::Duration};

use guide_capture::Monitor;

#[derive(Debug, Default, PartialEq)]
pub struct GuiOptions {
    pub delay: Duration,
    pub path: Option<PathBuf>,
}

pub const GUI_USAGE: &str = "usage: snapforge gui [--delay MILLISECONDS] [--path FOLDER]";

/// The monitor the selection screen goes on: the one holding the pointer, else the primary one,
/// else the first.
pub fn overlay_monitor(monitors: &[Monitor], pointer: Option<(i32, i32)>) -> Option<&Monitor> {
    pointer
        .and_then(|(x, y)| {
            monitors.iter().find(|m| {
                x >= m.x && y >= m.y && x < m.x + m.width as i32 && y < m.y + m.height as i32
            })
        })
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or_else(|| monitors.first())
}

/// Parses the arguments after `gui`.
pub fn parse_gui_args(args: &[String]) -> Result<GuiOptions, String> {
    let mut options = GuiOptions::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = |what: &str| {
            inline
                .clone()
                .or_else(|| it.next().cloned())
                .ok_or_else(|| format!("{name} needs {what}"))
        };
        match name {
            "--delay" | "-d" => {
                let text = value("a number of milliseconds")?;
                let ms: u64 = text
                    .parse()
                    .map_err(|_| format!("--delay needs a number of milliseconds, not {text:?}"))?;
                options.delay = Duration::from_millis(ms.min(600_000));
            }
            "--path" | "-p" => {
                let text = value("a folder")?;
                if text.is_empty() {
                    return Err("--path needs a folder".into());
                }
                options.path = Some(PathBuf::from(text));
            }
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<GuiOptions, String> {
        parse_gui_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn monitor(id: u32, x: i32, primary: bool) -> Monitor {
        Monitor {
            id,
            name: format!("m{id}"),
            x,
            y: 0,
            width: 1920,
            height: 1080,
            primary,
        }
    }

    #[test]
    fn the_selection_goes_on_the_monitor_with_the_pointer_else_the_primary_one() {
        let two = [monitor(1, 0, true), monitor(2, 1920, false)];
        assert_eq!(overlay_monitor(&two, Some((2500, 10))).unwrap().id, 2);
        assert_eq!(overlay_monitor(&two, Some((5000, 10))).unwrap().id, 1);
        assert_eq!(overlay_monitor(&two, None).unwrap().id, 1);
        let no_primary = [monitor(3, 0, false), monitor(4, 1920, false)];
        assert_eq!(overlay_monitor(&no_primary, None).unwrap().id, 3);
        assert!(overlay_monitor(&[], None).is_none());
    }

    #[test]
    fn no_arguments_give_the_defaults() {
        assert_eq!(parse(&[]).unwrap(), GuiOptions::default());
    }

    #[test]
    fn delay_and_path_are_read_in_both_spellings() {
        let o = parse(&["--delay", "1500", "--path", "/tmp/x"]).unwrap();
        assert_eq!(o.delay, Duration::from_millis(1500));
        assert_eq!(o.path, Some(PathBuf::from("/tmp/x")));
        let o = parse(&["-d", "20", "--path=/a b"]).unwrap();
        assert_eq!(
            (o.delay, o.path),
            (Duration::from_millis(20), Some(PathBuf::from("/a b")))
        );
    }

    #[test]
    fn bad_arguments_are_explained() {
        assert!(parse(&["--delay"]).unwrap_err().contains("needs"));
        assert!(parse(&["--delay", "soon"])
            .unwrap_err()
            .contains("milliseconds"));
        assert!(parse(&["--delay", "-5"]).is_err());
        assert!(parse(&["--path", ""]).is_err());
        assert!(parse(&["--wat"]).unwrap_err().contains("unknown option"));
        assert!(parse(&["extra"]).is_err());
    }

    #[test]
    fn an_absurd_delay_is_capped_at_ten_minutes() {
        assert_eq!(
            parse(&["--delay", "99999999999"]).unwrap().delay,
            Duration::from_secs(600)
        );
    }
}
