//! What it takes to take the same screenshot again.

use crate::{CaptureError, CaptureMode, Monitor, Rect, WindowInfo};

/// How the last screenshot was taken: enough to take it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastCapture {
    pub mode: CaptureMode,
    /// For `CaptureMode::Window`: the title, to find the window again when its id is gone.
    pub window_title: Option<String>,
}

impl LastCapture {
    pub fn new(mode: CaptureMode) -> Self {
        Self {
            mode,
            window_title: None,
        }
    }

    pub fn window(id: u32, title: &str) -> Self {
        Self {
            mode: CaptureMode::Window(id),
            window_title: Some(title.to_owned()),
        }
    }
}

/// Turns a remembered capture into a mode that works on the desktop as it is now.
///
/// - A region must still lie inside the monitors' combined area.
/// - A monitor must still exist.
/// - A window is found by id first, then by an exact title match; the id may have been reused
///   by another window, so a title that no longer matches the id's window counts as gone only
///   when the id is missing too.
pub fn resolve_repeat(
    last: &LastCapture,
    monitors: &[Monitor],
    windows: &[WindowInfo],
) -> Result<CaptureMode, CaptureError> {
    match &last.mode {
        CaptureMode::FullScreen => Ok(CaptureMode::FullScreen),
        CaptureMode::Monitor(id) => monitors
            .iter()
            .any(|m| m.id == *id)
            .then_some(CaptureMode::Monitor(*id))
            .ok_or(CaptureError::NoSuchMonitor(*id)),
        CaptureMode::Region(r) => {
            if inside_desktop(*r, monitors) {
                Ok(CaptureMode::Region(*r))
            } else {
                Err(CaptureError::InvalidRegion(*r))
            }
        }
        CaptureMode::Window(id) => {
            if windows.iter().any(|w| w.id == *id) {
                return Ok(CaptureMode::Window(*id));
            }
            last.window_title
                .as_deref()
                .filter(|t| !t.is_empty())
                .and_then(|t| windows.iter().find(|w| w.title == t))
                .map(|w| CaptureMode::Window(w.id))
                .ok_or(CaptureError::NoSuchWindow(*id))
        }
    }
}

/// Whether `r` fits in the bounding box of the monitors (the size of the full capture).
/// With no monitors listed there is nothing to check against, so it passes.
fn inside_desktop(r: Rect, monitors: &[Monitor]) -> bool {
    if monitors.is_empty() {
        return true;
    }
    let right = monitors
        .iter()
        .map(|m| i64::from(m.x) + i64::from(m.width))
        .max()
        .unwrap_or(0);
    let bottom = monitors
        .iter()
        .map(|m| i64::from(m.y) + i64::from(m.height))
        .max()
        .unwrap_or(0);
    r.w > 0
        && r.h > 0
        && i64::from(r.x) + i64::from(r.w) <= right
        && i64::from(r.y) + i64::from(r.h) <= bottom
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IRect;

    fn monitor(id: u32, x: i32, w: u32) -> Monitor {
        Monitor {
            id,
            name: format!("M{id}"),
            x,
            y: 0,
            width: w,
            height: 1080,
            primary: id == 1,
        }
    }

    fn window(id: u32, title: &str) -> WindowInfo {
        WindowInfo {
            id,
            title: title.into(),
            rect: IRect {
                x: 0,
                y: 0,
                w: 100,
                h: 100,
            },
        }
    }

    fn rect(x: u32, y: u32, w: u32, h: u32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn full_screen_is_always_repeatable() {
        let last = LastCapture::new(CaptureMode::FullScreen);
        assert_eq!(resolve_repeat(&last, &[], &[]), Ok(CaptureMode::FullScreen));
    }

    #[test]
    fn a_monitor_that_is_gone_cannot_be_repeated() {
        let last = LastCapture::new(CaptureMode::Monitor(2));
        let one = [monitor(1, 0, 1920)];
        assert_eq!(
            resolve_repeat(&last, &one, &[]),
            Err(CaptureError::NoSuchMonitor(2))
        );
        let two = [monitor(1, 0, 1920), monitor(2, 1920, 1920)];
        assert_eq!(
            resolve_repeat(&last, &two, &[]),
            Ok(CaptureMode::Monitor(2))
        );
    }

    #[test]
    fn a_region_must_still_fit_on_the_desktop() {
        let ms = [monitor(1, 0, 1920)];
        let ok = LastCapture::new(CaptureMode::Region(rect(100, 100, 1820, 980)));
        assert!(resolve_repeat(&ok, &ms, &[]).is_ok());
        let off = LastCapture::new(CaptureMode::Region(rect(1900, 0, 100, 100)));
        assert_eq!(
            resolve_repeat(&off, &ms, &[]),
            Err(CaptureError::InvalidRegion(rect(1900, 0, 100, 100)))
        );
    }

    #[test]
    fn a_region_on_a_second_monitor_fits_while_it_is_still_connected() {
        let r = rect(2000, 0, 300, 300);
        let last = LastCapture::new(CaptureMode::Region(r));
        let both = [monitor(1, 0, 1920), monitor(2, 1920, 1920)];
        assert_eq!(
            resolve_repeat(&last, &both, &[]),
            Ok(CaptureMode::Region(r))
        );
        assert!(resolve_repeat(&last, &both[..1], &[]).is_err());
    }

    #[test]
    fn a_window_is_found_by_its_id() {
        let last = LastCapture::window(7, "Editor");
        let ws = [window(3, "Other"), window(7, "Editor - changed")];
        assert_eq!(resolve_repeat(&last, &[], &ws), Ok(CaptureMode::Window(7)));
    }

    #[test]
    fn a_window_with_a_new_id_is_found_by_its_title() {
        let last = LastCapture::window(7, "Editor");
        let ws = [window(3, "Other"), window(9, "Editor")];
        assert_eq!(resolve_repeat(&last, &[], &ws), Ok(CaptureMode::Window(9)));
    }

    #[test]
    fn a_window_that_is_gone_cannot_be_repeated() {
        let last = LastCapture::window(7, "Editor");
        let ws = [window(3, "Other")];
        assert_eq!(
            resolve_repeat(&last, &[], &ws),
            Err(CaptureError::NoSuchWindow(7))
        );
        let untitled = LastCapture::window(7, "");
        assert_eq!(
            resolve_repeat(&untitled, &[], &[window(9, "")]),
            Err(CaptureError::NoSuchWindow(7))
        );
    }
}
