/// X11 keysym for a canonical key name (see `guide_core::Shortcut::key`).
pub fn keysym_for(name: &str) -> Option<u32> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        // Letters use their lower-case keysym; digits and ASCII map to themselves.
        return c
            .is_ascii_alphanumeric()
            .then(|| u32::from(c.to_ascii_lowercase()));
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=12).contains(&n).then(|| 0xFFBE + n - 1);
    }
    Some(match name {
        "Print" => 0xFF61,
        "Space" => 0x20,
        "Return" => 0xFF0D,
        "Tab" => 0xFF09,
        "Left" => 0xFF51,
        "Up" => 0xFF52,
        "Right" => 0xFF53,
        "Down" => 0xFF54,
        "Home" => 0xFF50,
        "PageUp" => 0xFF55,
        "PageDown" => 0xFF56,
        "End" => 0xFF57,
        "Insert" => 0xFF63,
        "Delete" => 0xFFFF,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use guide_core::KEY_NAMES;

    #[test]
    fn every_core_key_name_has_a_keysym() {
        for k in KEY_NAMES
            .iter()
            .chain(["Home", "End", "PageUp", "PageDown"].iter())
        {
            assert!(keysym_for(k).is_some(), "{k}");
        }
        for c in ('A'..='Z').chain('0'..='9') {
            assert!(keysym_for(&c.to_string()).is_some());
        }
    }

    #[test]
    fn known_values() {
        assert_eq!(keysym_for("R"), Some(0x72));
        assert_eq!(keysym_for("F1"), Some(0xFFBE));
        assert_eq!(keysym_for("F12"), Some(0xFFC9));
        assert_eq!(keysym_for("Print"), Some(0xFF61));
        assert_eq!(keysym_for("F13"), None);
        assert_eq!(keysym_for("Banana"), None);
    }
}
