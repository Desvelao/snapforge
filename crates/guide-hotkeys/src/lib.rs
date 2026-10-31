//! Global keyboard shortcuts behind a UI-independent interface.
//!
//! The UI only sees [`HotkeyBackend`]; X11 details stay in the `x11` module.

mod keysym;
mod pointer;
mod x11;

use guide_core::{Action, Shortcut};

pub use keysym::keysym_for;
pub use pointer::{detect_pointer, ClickEvent, PointerBackend, UnsupportedPointer, X11Pointer};
pub use x11::X11Hotkeys;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HotkeyError {
    #[error("already used by another application")]
    InUse,
    #[error("the key \"{0}\" does not exist on this keyboard layout")]
    KeyNotOnKeyboard(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Backend(String),
}

pub trait HotkeyBackend: Send {
    fn name(&self) -> &str;

    /// `Err(reason)` when this session cannot do global shortcuts at all.
    fn availability(&self) -> Result<(), String>;

    /// Replaces every previously registered shortcut. One result per binding,
    /// so a conflict on one shortcut does not affect the others.
    fn register(
        &mut self,
        bindings: &[(Action, Shortcut)],
    ) -> Vec<(Action, Result<(), HotkeyError>)>;

    /// Next shortcut press, if any.
    fn try_recv(&self) -> Option<Action>;
}

/// Backend for sessions where global shortcuts are not possible; every
/// registration fails with `reason`.
pub struct UnsupportedHotkeys {
    name: String,
    reason: String,
}

impl HotkeyBackend for UnsupportedHotkeys {
    fn name(&self) -> &str {
        &self.name
    }

    fn availability(&self) -> Result<(), String> {
        Err(self.reason.clone())
    }

    fn register(
        &mut self,
        bindings: &[(Action, Shortcut)],
    ) -> Vec<(Action, Result<(), HotkeyError>)> {
        bindings
            .iter()
            .map(|(a, _)| (*a, Err(HotkeyError::Unsupported(self.reason.clone()))))
            .collect()
    }

    fn try_recv(&self) -> Option<Action> {
        None
    }
}

/// Picks a backend from the session environment (same order as the capture
/// backends: Wayland first, because X11 grabs do not work there).
/// `notify` is called from a background thread whenever a shortcut fires.
pub fn detect(notify: impl Fn() + Send + Sync + 'static) -> Box<dyn HotkeyBackend> {
    detect_from(
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
        notify,
    )
}

fn detect_from(
    wayland: bool,
    x11: bool,
    notify: impl Fn() + Send + Sync + 'static,
) -> Box<dyn HotkeyBackend> {
    let unsupported = |name: &str, reason: &str| -> Box<dyn HotkeyBackend> {
        Box::new(UnsupportedHotkeys {
            name: name.into(),
            reason: reason.into(),
        })
    };
    if wayland {
        return unsupported(
            "Wayland",
            "global shortcuts on Wayland are not implemented yet",
        );
    }
    if x11 {
        return match X11Hotkeys::connect(notify) {
            Ok(b) => Box::new(b),
            Err(e) => Box::new(UnsupportedHotkeys {
                name: "X11".into(),
                reason: e.to_string(),
            }),
        };
    }
    unsupported("none", "no graphical session found")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_reports_unsupported_per_binding() {
        let mut b = detect_from(true, true, || {});
        assert_eq!(b.name(), "Wayland");
        assert!(b.availability().is_err());
        let out = b.register(&[(
            Action::CaptureRegion,
            Action::CaptureRegion.default_shortcut(),
        )]);
        assert!(matches!(out[0].1, Err(HotkeyError::Unsupported(_))));
        assert_eq!(b.try_recv(), None);
    }

    #[test]
    fn no_session_is_unsupported() {
        assert!(detect_from(false, false, || {}).availability().is_err());
    }
}
