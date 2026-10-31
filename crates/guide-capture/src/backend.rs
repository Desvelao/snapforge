use crate::{
    visible_rect, x11::X11Backend, Capabilities, CaptureError, CaptureMode, CaptureRequest,
    Monitor, Screenshot, Support, WindowInfo,
};

pub trait CaptureBackend: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    fn monitors(&self) -> Result<Vec<Monitor>, CaptureError>;
    /// Top-level windows, bottom of the stack first.
    fn windows(&self) -> Result<Vec<WindowInfo>, CaptureError> {
        Err(CaptureError::Unsupported(
            "window listing is not supported by this backend".into(),
        ))
    }
    /// Where the pointer is, in desktop pixels, if the backend can tell.
    fn pointer_position(&self) -> Option<(i32, i32)> {
        None
    }
    fn capture(&self, request: &CaptureRequest) -> Result<Screenshot, CaptureError>;
}

/// Backend used when no supported display server is usable; every capture
/// fails with `reason`, which is shown to the user.
pub struct UnavailableBackend {
    name: String,
    reason: String,
}

impl UnavailableBackend {
    pub fn new(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reason: reason.into(),
        }
    }
}

impl CaptureBackend for UnavailableBackend {
    fn capabilities(&self) -> Capabilities {
        let no = || Support::No(self.reason.clone());
        Capabilities {
            backend: self.name.clone(),
            full_screen: no(),
            monitor_select: no(),
            region: no(),
            window: no(),
            cursor: no(),
        }
    }

    fn monitors(&self) -> Result<Vec<Monitor>, CaptureError> {
        Err(CaptureError::Unsupported(self.reason.clone()))
    }

    fn windows(&self) -> Result<Vec<WindowInfo>, CaptureError> {
        Err(CaptureError::Unsupported(self.reason.clone()))
    }

    fn capture(&self, _request: &CaptureRequest) -> Result<Screenshot, CaptureError> {
        Err(CaptureError::Unsupported(self.reason.clone()))
    }
}

/// Picks a backend from the session environment. Wayland sessions are checked
/// first because capturing through XWayland yields blank images.
pub fn detect_backend() -> Box<dyn CaptureBackend> {
    detect_from(
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
    )
}

fn detect_from(wayland: bool, x11: bool) -> Box<dyn CaptureBackend> {
    if wayland {
        return Box::new(UnavailableBackend::new(
            "Wayland",
            "Wayland capture (XDG Desktop Portal) is not implemented yet",
        ));
    }
    if x11 {
        return match X11Backend::connect() {
            Ok(b) => Box::new(b),
            Err(e) => Box::new(UnavailableBackend::new("X11", e.to_string())),
        };
    }
    Box::new(UnavailableBackend::new(
        "none",
        "no graphical session found (neither WAYLAND_DISPLAY nor DISPLAY is set)",
    ))
}

/// Shared by backends: crops a full-desktop image to the requested mode.
pub(crate) fn apply_mode(
    full: Screenshot,
    monitors: &[Monitor],
    windows: &[WindowInfo],
    mode: &CaptureMode,
) -> Result<Screenshot, CaptureError> {
    match mode {
        CaptureMode::FullScreen => Ok(full),
        CaptureMode::Region(rect) => full.crop(*rect),
        CaptureMode::Window(id) => {
            let w = windows
                .iter()
                .find(|w| w.id == *id)
                .ok_or(CaptureError::NoSuchWindow(*id))?;
            let rect = visible_rect(w.rect, (full.width, full.height))
                .ok_or(CaptureError::NoSuchWindow(*id))?;
            full.crop(rect)
        }
        CaptureMode::Monitor(id) => {
            let m = monitors
                .iter()
                .find(|m| m.id == *id)
                .ok_or(CaptureError::NoSuchMonitor(*id))?;
            full.crop(crate::Rect {
                x: m.x.max(0) as u32,
                y: m.y.max(0) as u32,
                w: m.width,
                h: m.height,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_session_reports_reason() {
        let b = detect_from(false, false);
        let caps = b.capabilities();
        assert!(matches!(caps.full_screen, Support::No(ref r) if r.contains("DISPLAY")));
        assert!(matches!(
            b.capture(&CaptureRequest {
                include_cursor: false,
                mode: CaptureMode::FullScreen
            }),
            Err(CaptureError::Unsupported(_))
        ));
    }

    #[test]
    fn wayland_takes_priority_over_xwayland() {
        let b = detect_from(true, true);
        assert_eq!(b.capabilities().backend, "Wayland");
    }
}
