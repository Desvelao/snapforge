//! Global mouse-click events.
//!
//! Uses XInput2 raw button events on the root window: they are passive (the
//! click still reaches the application under the pointer) and also fire for
//! XTEST input, which the end-to-end tests rely on.

use std::{
    sync::mpsc::{self, Receiver},
    thread,
};

use x11rb::{
    connection::Connection,
    protocol::{
        xinput::{self, ConnectionExt as _, XIEventMask},
        xproto::{ConnectionExt as _, Window},
        Event,
    },
    rust_connection::RustConnection,
};

use crate::HotkeyError;

/// A finished mouse click (button release), in desktop pixels.
///
/// The position is read from the server right after the release event arrives, so a pointer that
/// moves in that instant is reported where it is by then, not where it was released.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClickEvent {
    pub x: i32,
    pub y: i32,
    /// X11 button number: 1 left, 2 middle, 3 right.
    pub button: u8,
}

pub trait PointerBackend: Send {
    /// `Err(reason)` when this session cannot report clicks at all.
    fn availability(&self) -> Result<(), String>;

    /// Next click, if any.
    fn try_recv(&self) -> Option<ClickEvent>;
}

/// Backend for sessions where global clicks cannot be observed.
pub struct UnsupportedPointer {
    reason: String,
}

impl PointerBackend for UnsupportedPointer {
    fn availability(&self) -> Result<(), String> {
        Err(self.reason.clone())
    }

    fn try_recv(&self) -> Option<ClickEvent> {
        None
    }
}

pub struct X11Pointer {
    rx: Receiver<ClickEvent>,
}

impl X11Pointer {
    pub fn connect(notify: impl Fn() + Send + Sync + 'static) -> Result<Self, HotkeyError> {
        let backend = |e: &dyn std::fmt::Display| HotkeyError::Backend(e.to_string());
        let (conn, screen) = x11rb::connect(None)
            .map_err(|e| HotkeyError::Backend(format!("cannot connect to the X server: {e}")))?;
        let root = conn.setup().roots[screen].root;
        let version = conn
            .xinput_xi_query_version(2, 0)
            .map_err(|e| backend(&e))?
            .reply()
            .map_err(|_| HotkeyError::Unsupported("the X server has no XInput2".into()))?;
        if version.major_version < 2 {
            return Err(HotkeyError::Unsupported(
                "the X server has no XInput2".into(),
            ));
        }
        conn.xinput_xi_select_events(
            root,
            &[xinput::EventMask {
                // XIAllDevices: with a window manager running, selecting only the
                // master devices delivered no raw events at all.
                deviceid: 0,
                mask: vec![XIEventMask::RAW_BUTTON_RELEASE],
            }],
        )
        .map_err(|e| backend(&e))?
        .check()
        .map_err(|e| backend(&e))?;
        conn.flush().map_err(|e| backend(&e))?;

        let (tx, rx) = mpsc::channel();
        // Lives until the process exits; it only blocks on the X connection.
        thread::spawn(move || {
            while let Ok(event) = conn.wait_for_event() {
                let Event::XinputRawButtonRelease(release) = event else {
                    continue;
                };
                // Buttons 4-7 are the scroll wheel.
                let Ok(button) = u8::try_from(release.detail) else {
                    continue;
                };
                if !(1..=3).contains(&button) {
                    continue;
                }
                let Some((x, y)) = pointer_position(&conn, root) else {
                    continue;
                };
                if tx.send(ClickEvent { x, y, button }).is_err() {
                    break;
                }
                notify();
            }
        });
        Ok(Self { rx })
    }
}

fn pointer_position(conn: &RustConnection, root: Window) -> Option<(i32, i32)> {
    let p = conn.query_pointer(root).ok()?.reply().ok()?;
    Some((i32::from(p.root_x), i32::from(p.root_y)))
}

impl PointerBackend for X11Pointer {
    fn availability(&self) -> Result<(), String> {
        Ok(())
    }

    fn try_recv(&self) -> Option<ClickEvent> {
        self.rx.try_recv().ok()
    }
}

/// Picks a backend from the session environment, like [`crate::detect`].
pub fn detect_pointer(notify: impl Fn() + Send + Sync + 'static) -> Box<dyn PointerBackend> {
    detect_pointer_from(
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
        notify,
    )
}

fn detect_pointer_from(
    wayland: bool,
    x11: bool,
    notify: impl Fn() + Send + Sync + 'static,
) -> Box<dyn PointerBackend> {
    let unsupported =
        |reason: String| -> Box<dyn PointerBackend> { Box::new(UnsupportedPointer { reason }) };
    if wayland {
        return unsupported("click capture on Wayland is not implemented yet".into());
    }
    if x11 {
        return match X11Pointer::connect(notify) {
            Ok(b) => Box::new(b),
            Err(e) => unsupported(e.to_string()),
        };
    }
    unsupported("no graphical session found".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_reports_unsupported() {
        let b = detect_pointer_from(true, true, || {});
        assert!(b.availability().is_err());
        assert_eq!(b.try_recv(), None);
    }

    #[test]
    fn no_session_is_unsupported() {
        assert!(detect_pointer_from(false, false, || {})
            .availability()
            .is_err());
    }
}
