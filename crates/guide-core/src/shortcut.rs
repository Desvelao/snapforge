use std::fmt;

/// Things a global shortcut can trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    CaptureFullScreen,
    CaptureRegion,
    CaptureWindow,
    /// Starts or stops a session that captures on every mouse click.
    ToggleClickCapture,
}

impl Action {
    pub const ALL: [Action; 4] = [
        Action::CaptureFullScreen,
        Action::CaptureRegion,
        Action::CaptureWindow,
        Action::ToggleClickCapture,
    ];

    /// The English name used in diagnostics: `--self-test` output and settings repair notes (which
    /// scripts read). It is never translated; the window shows the name through `tr` (see
    /// `action_label` in `guide-app/src/app.rs`).
    pub fn label(self) -> &'static str {
        match self {
            Action::CaptureFullScreen => "Capture full screen",
            Action::CaptureRegion => "Capture region",
            Action::CaptureWindow => "Capture window",
            Action::ToggleClickCapture => "Start or stop click capture",
        }
    }

    /// Print is deliberately not used: desktops such as XFCE already bind it.
    pub fn default_shortcut(self) -> Shortcut {
        let key = match self {
            Action::CaptureFullScreen => "F",
            Action::CaptureRegion => "R",
            Action::CaptureWindow => "W",
            Action::ToggleClickCapture => "C",
        };
        Shortcut {
            ctrl: true,
            shift: true,
            alt: false,
            super_: false,
            key: key.into(),
        }
    }
}

/// Key names a shortcut may use, besides single letters and digits.
pub const KEY_NAMES: [&str; 22] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "Print", "Space",
    "Return", "Tab", "Left", "Right", "Up", "Down", "Insert", "Delete",
];
const EXTRA_KEYS: [&str; 4] = ["Home", "End", "PageUp", "PageDown"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShortcutError {
    Empty,
    NoKey,
    UnknownKey(String),
    /// Two `+` in a row, or a `+` at the start: a part between them is empty.
    EmptyModifier,
    UnknownModifier(String),
    DuplicateModifier(String),
    NoModifier,
}

impl fmt::Display for ShortcutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShortcutError::Empty => write!(f, "the shortcut is empty"),
            ShortcutError::NoKey => write!(f, "the shortcut has no key"),
            ShortcutError::UnknownKey(k) => write!(f, "unknown key \"{k}\""),
            ShortcutError::EmptyModifier => write!(
                f,
                "the shortcut has an empty part (two \"+\" in a row); \"+\" itself cannot be used as a key"
            ),
            ShortcutError::UnknownModifier(m) => write!(f, "unknown modifier \"{m}\""),
            ShortcutError::DuplicateModifier(m) => write!(f, "modifier \"{m}\" is repeated"),
            ShortcutError::NoModifier => write!(
                f,
                "a global shortcut needs at least one of Ctrl, Alt, Shift or Super"
            ),
        }
    }
}

impl std::error::Error for ShortcutError {}

/// A modifier combination plus one key, e.g. `Ctrl+Shift+R`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_: bool,
    /// Canonical key name: `A`-`Z`, `0`-`9`, or one of [`KEY_NAMES`] and `Home`/`End`/`PageUp`/`PageDown`.
    pub key: String,
}

/// Maps user input (any case, a few aliases) to a canonical key name.
pub fn canonical_key(input: &str) -> Option<String> {
    let input = input.trim();
    let mut chars = input.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return c
            .is_ascii_alphanumeric()
            .then(|| c.to_ascii_uppercase().to_string());
    }
    if input.eq_ignore_ascii_case("enter") {
        return Some("Return".into());
    }
    KEY_NAMES
        .iter()
        .chain(EXTRA_KEYS.iter())
        .find(|k| k.eq_ignore_ascii_case(input))
        .map(|k| (*k).to_string())
}

impl Shortcut {
    pub fn parse(text: &str) -> Result<Shortcut, ShortcutError> {
        if text.trim().is_empty() {
            return Err(ShortcutError::Empty);
        }
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (key_part, mods) = parts.split_last().expect("split yields at least one part");
        if key_part.is_empty() {
            return Err(ShortcutError::NoKey);
        }
        let mut s = Shortcut {
            ctrl: false,
            alt: false,
            shift: false,
            super_: false,
            key: String::new(),
        };
        for m in mods {
            if m.is_empty() {
                return Err(ShortcutError::EmptyModifier);
            }
            let slot = match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut s.ctrl,
                "alt" => &mut s.alt,
                "shift" => &mut s.shift,
                "super" | "win" | "meta" | "cmd" => &mut s.super_,
                _ => return Err(ShortcutError::UnknownModifier((*m).to_string())),
            };
            if *slot {
                return Err(ShortcutError::DuplicateModifier((*m).to_string()));
            }
            *slot = true;
        }
        s.key = canonical_key(key_part)
            .ok_or_else(|| ShortcutError::UnknownKey((*key_part).to_string()))?;
        if !(s.ctrl || s.alt || s.shift || s.super_) {
            return Err(ShortcutError::NoModifier);
        }
        Ok(s)
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.super_, "Super"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

/// Returns the first two actions bound to the same shortcut, if any.
pub fn find_duplicate(bindings: &[(Action, Shortcut)]) -> Option<(Action, Action)> {
    for (i, (a, sa)) in bindings.iter().enumerate() {
        if let Some((b, _)) = bindings[i + 1..].iter().find(|(_, sb)| sb == sa) {
            return Some((*a, *b));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_and_round_trip() {
        for a in Action::ALL {
            let d = a.default_shortcut();
            assert_eq!(Shortcut::parse(&d.to_string()), Ok(d));
        }
        assert_eq!(
            Action::CaptureRegion.default_shortcut().to_string(),
            "Ctrl+Shift+R"
        );
    }

    #[test]
    fn parsing_is_case_and_alias_tolerant_and_normalises_display() {
        let s = Shortcut::parse(" control + SHIFT + enter ").unwrap();
        assert_eq!(s.to_string(), "Ctrl+Shift+Return");
        assert_eq!(
            Shortcut::parse("win+alt+f5").unwrap().to_string(),
            "Alt+Super+F5"
        );
        assert_eq!(Shortcut::parse("ctrl+shift+a").unwrap().key, "A");
    }

    #[test]
    fn every_listed_key_name_is_accepted() {
        for k in KEY_NAMES.iter().chain(EXTRA_KEYS.iter()) {
            assert_eq!(canonical_key(k).as_deref(), Some(*k));
            assert!(Shortcut::parse(&format!("Ctrl+{k}")).is_ok(), "{k}");
        }
        for c in ('A'..='Z').chain('0'..='9') {
            assert!(Shortcut::parse(&format!("Alt+{c}")).is_ok(), "{c}");
        }
    }

    #[test]
    fn invalid_input_is_rejected_with_a_reason() {
        let e = |s| Shortcut::parse(s).unwrap_err();
        assert_eq!(e(""), ShortcutError::Empty);
        assert_eq!(e("  "), ShortcutError::Empty);
        assert_eq!(e("R"), ShortcutError::NoModifier);
        assert_eq!(e("F5"), ShortcutError::NoModifier);
        assert_eq!(e("Ctrl+"), ShortcutError::NoKey);
        assert_eq!(e("Ctrl++R"), ShortcutError::EmptyModifier);
        assert_eq!(e("+R"), ShortcutError::EmptyModifier);
        assert_eq!(e("Ctrl+Banana"), ShortcutError::UnknownKey("Banana".into()));
        assert_eq!(e("Hyper+R"), ShortcutError::UnknownModifier("Hyper".into()));
        assert_eq!(
            e("Ctrl+ctrl+R"),
            ShortcutError::DuplicateModifier("ctrl".into())
        );
        assert_eq!(e("Ctrl+F13"), ShortcutError::UnknownKey("F13".into()));
        assert_eq!(e("Ctrl+é"), ShortcutError::UnknownKey("é".into()));
    }

    #[test]
    fn duplicates_are_found() {
        let s = |t: &str| Shortcut::parse(t).unwrap();
        let ok = [
            (Action::CaptureFullScreen, s("Ctrl+F")),
            (Action::CaptureRegion, s("Ctrl+R")),
        ];
        assert_eq!(find_duplicate(&ok), None);
        let dup = [
            (Action::CaptureFullScreen, s("Ctrl+F")),
            (Action::CaptureRegion, s("Ctrl+R")),
            (Action::CaptureWindow, s("control+f")),
        ];
        assert_eq!(
            find_duplicate(&dup),
            Some((Action::CaptureFullScreen, Action::CaptureWindow))
        );
    }
}
