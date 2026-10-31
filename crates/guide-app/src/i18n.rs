//! Translation of the user interface.
//!
//! The English text itself is the message key (as in gettext): `tr("Save")` returns "Save" unless a
//! catalog for the current language has an entry for it. This keeps the code readable, makes adding
//! a string a one-line change, and lets a missing translation fall back to English instead of
//! showing a key. `tests/i18n_guard.rs` fails the build if user-visible text bypasses this module.
//!
//! * `tr("Save")` - a fixed text.
//! * `tr_fmt("Saved {path}", &[("path", &p)])` - a text with named placeholders.
//! * `tr_n("{n} step", "{n} steps", n)` - singular and plural; translations may have more forms.
//!
//! Catalogs are JSON files in `locales/<code>.json` mapping the English text to its translation
//! (or, for plurals, to a list of forms). `locales/template.json` lists every message; regenerate
//! it with `UPDATE_I18N=1 cargo test -p guide-app --test i18n_guard`. The special language
//! `pseudo` (set `SNAPFORGE_LANG=pseudo`) turns every message into accented, padded text so
//! untranslated or truncated text is easy to spot on screen.

use std::{collections::HashMap, fmt::Display, sync::RwLock};

/// Languages with an embedded catalog (English is the source language and needs none).
/// Add `("de", include_str!("../locales/de.json"))` here when a translation exists.
const CATALOGS: &[(&str, &str)] = &[];

pub const PSEUDO: &str = "pseudo";

/// The entry of a catalog that holds the language's own name, e.g. `"_language": "Deutsch"`.
pub const LANGUAGE_KEY: &str = "_language";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    One(&'static str),
    Forms(Vec<&'static str>),
}

/// The messages of one language.
#[derive(Debug, Default)]
pub struct Catalog {
    lang: String,
    pseudo: bool,
    map: HashMap<String, Entry>,
}

/// Index of the plural form to use for `n` in `lang`. Languages that need more than two forms
/// (Russian, Polish, Arabic...) must extend this before shipping a catalog for them.
pub fn plural_index(lang: &str, n: usize) -> usize {
    match lang {
        // Zero and one are singular.
        "fr" | "pt-BR" | "hi" => usize::from(n > 1),
        // One singular form only.
        "ja" | "zh" | "ko" | "vi" | "th" | "id" => 0,
        _ => usize::from(n != 1),
    }
}

/// Makes `s` look foreign but readable, longer than the original, and wrapped in brackets,
/// keeping `{placeholders}` untouched.
pub fn pseudo(s: &str) -> String {
    let mut out = String::from("[");
    let mut in_placeholder = false;
    for c in s.chars() {
        out.push(match c {
            '{' => {
                in_placeholder = true;
                c
            }
            '}' => {
                in_placeholder = false;
                c
            }
            _ if in_placeholder => c,
            'a' => 'à',
            'e' => 'é',
            'i' => 'î',
            'o' => 'ö',
            'u' => 'ü',
            'c' => 'ç',
            'n' => 'ñ',
            'y' => 'ý',
            'A' => 'Å',
            'E' => 'É',
            'I' => 'Î',
            'O' => 'Ö',
            'U' => 'Û',
            other => other,
        });
    }
    // About a third longer, like many real translations.
    out.push_str(&"~".repeat((s.chars().count() / 3).max(1)));
    out.push(']');
    out
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

impl Catalog {
    /// Parses a catalog. Entries that are not a string or a list of strings are ignored.
    pub fn from_json(lang: &str, json: &str) -> Result<Catalog, String> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let obj = value.as_object().ok_or("a catalog must be a JSON object")?;
        let mut map = HashMap::new();
        for (msgid, v) in obj {
            if msgid == LANGUAGE_KEY {
                continue; // the language's name, not a message
            }
            let entry = match v {
                serde_json::Value::String(s) if !s.is_empty() => Entry::One(leak(s.clone())),
                serde_json::Value::Array(a) => {
                    let forms: Vec<&'static str> = a
                        .iter()
                        .filter_map(|f| {
                            f.as_str()
                                .filter(|s| !s.is_empty())
                                .map(|s| leak(s.to_string()))
                        })
                        .collect();
                    if forms.is_empty() {
                        continue;
                    }
                    Entry::Forms(forms)
                }
                _ => continue, // an untranslated (empty) entry falls back to English
            };
            map.insert(msgid.clone(), entry);
        }
        Ok(Catalog {
            lang: lang.to_string(),
            pseudo: false,
            map,
        })
    }

    pub fn pseudo() -> Catalog {
        Catalog {
            lang: PSEUDO.into(),
            pseudo: true,
            map: HashMap::new(),
        }
    }

    fn entry(&self, msgid: &str) -> Option<Entry> {
        self.map.get(msgid).cloned()
    }

    pub fn translate(&mut self, msgid: &'static str) -> &'static str {
        if let Some(Entry::One(s)) = self.map.get(msgid) {
            return s;
        }
        if self.pseudo {
            let s = leak(pseudo(msgid));
            self.map.insert(msgid.to_string(), Entry::One(s));
            return s;
        }
        msgid
    }

    /// The plural form of `singular` for `n`, or `None` to use the English words.
    pub fn plural(&self, singular: &str, n: usize) -> Option<&'static str> {
        match self.entry(singular)? {
            Entry::Forms(forms) => forms
                .get(plural_index(&self.lang, n))
                .or(forms.last())
                .copied(),
            Entry::One(s) => Some(s),
        }
    }
}

static CURRENT: RwLock<Option<Catalog>> = RwLock::new(None);

/// Switches the language. `""` and `"en"` select English. Returns `false` (and keeps English)
/// if there is no catalog for `code`.
pub fn set_language(code: &str) -> bool {
    let code = code.trim();
    let catalog = if code.is_empty() || code == "en" {
        None
    } else if code == PSEUDO {
        Some(Catalog::pseudo())
    } else {
        let Some((_, json)) = CATALOGS.iter().find(|(c, _)| *c == code) else {
            *CURRENT.write().expect("language lock") = None;
            return false;
        };
        match Catalog::from_json(code, json) {
            Ok(c) => Some(c),
            Err(_) => {
                *CURRENT.write().expect("language lock") = None;
                return false;
            }
        }
    };
    *CURRENT.write().expect("language lock") = catalog;
    true
}

/// The language to use: `SNAPFORGE_LANG`, then the saved choice, then the desktop's.
pub fn detect(saved: &str) -> String {
    let overridden = std::env::var("SNAPFORGE_LANG").ok();
    if let Some(l) = overridden {
        if !l.trim().is_empty() {
            return l.trim().to_string();
        }
    }
    if !saved.trim().is_empty() {
        return saved.trim().to_string();
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let primary = v
                .split(':')
                .next()
                .unwrap_or("")
                .split(['.', '@'])
                .next()
                .unwrap_or("");
            let code = primary.split('_').next().unwrap_or("");
            if !code.is_empty() && code != "C" && code != "POSIX" {
                return code.to_string();
            }
        }
    }
    String::new()
}

/// The languages this build can show, with English first.
pub fn available() -> Vec<&'static str> {
    let mut v = vec!["en"];
    v.extend(CATALOGS.iter().map(|(c, _)| *c));
    v
}

/// The name of a language in that language, for the language picker. Falls back to the code.
pub fn native_name(code: &str) -> String {
    if code == "en" {
        return "English".into();
    }
    CATALOGS
        .iter()
        .find(|(c, _)| *c == code)
        .and_then(|(_, json)| name_in_catalog(json))
        .unwrap_or_else(|| code.to_string())
}

fn name_in_catalog(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let name = value.get(LANGUAGE_KEY)?.as_str()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Translates a fixed message.
pub fn tr(msgid: &'static str) -> &'static str {
    // Fast path: English, or a catalog that only needs a read lock.
    {
        let guard = CURRENT.read().expect("language lock");
        match guard.as_ref() {
            None => return msgid,
            Some(c) if !c.pseudo => {
                return match c.map.get(msgid) {
                    Some(Entry::One(s)) => s,
                    _ => msgid,
                };
            }
            Some(_) => {
                if let Some(Entry::One(s)) = guard.as_ref().and_then(|c| c.map.get(msgid)) {
                    return s;
                }
            }
        }
    }
    CURRENT
        .write()
        .expect("language lock")
        .as_mut()
        .map_or(msgid, |c| c.translate(msgid))
}

/// Translates a message and fills its `{name}` placeholders.
pub fn tr_fmt(msgid: &'static str, args: &[(&str, &dyn Display)]) -> String {
    fill(tr(msgid), args)
}

/// `singular` when `n` is 1 and `plural` otherwise, in the current language's rule. `{n}` in
/// either is replaced by the number; other placeholders go in `args`.
pub fn tr_n(
    singular: &'static str,
    plural: &'static str,
    n: usize,
    args: &[(&str, &dyn Display)],
) -> String {
    let text = {
        let guard = CURRENT.read().expect("language lock");
        match guard.as_ref() {
            Some(c) if !c.pseudo => c.plural(singular, n),
            _ => None,
        }
    };
    let text = match text {
        Some(t) => t,
        None if is_pseudo() => tr(if n == 1 { singular } else { plural }),
        None => {
            if n == 1 {
                singular
            } else {
                plural
            }
        }
    };
    let mut all: Vec<(&str, &dyn Display)> = vec![("n", &n)];
    all.extend_from_slice(args);
    fill(text, &all)
}

fn is_pseudo() -> bool {
    CURRENT
        .read()
        .expect("language lock")
        .as_ref()
        .is_some_and(|c| c.pseudo)
}

/// Replaces each `{name}` with its value. A placeholder with no value stays as written.
pub fn fill(text: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, v)) => out.push_str(&v.to_string()),
                    None => {
                        // A message naming a value the caller did not pass is a programming error
                        // (the screen would show a raw `{name}`); tests and debug runs catch it.
                        debug_assert!(
                            name.is_empty()
                                || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                            "placeholder {{{name}}} has no value in {text:?}"
                        );
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The placeholder names (`{name}`) a message uses, in order of appearance.
#[cfg(test)]
pub fn placeholders(text: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut rest = text;
    while let Some(s) = rest.find('{') {
        let after = &rest[s + 1..];
        let Some(e) = after.find('}') else { break };
        v.push(after[..e].to_string());
        rest = &after[e + 1..];
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const DE: &str = r#"{
        "Save": "Speichern",
        "Saved {path}": "{path} gespeichert",
        "{n} step": ["{n} Schritt", "{n} Schritte"],
        "Untranslated": "",
        "Weird": 42
    }"#;

    #[test]
    fn a_language_is_named_in_itself_and_the_name_is_not_a_message() {
        assert_eq!(native_name("en"), "English");
        assert_eq!(native_name("xx"), "xx", "unknown codes show as they are");
        let with_name = r#"{"_language": "Deutsch", "Save": "Speichern"}"#;
        assert_eq!(name_in_catalog(with_name).as_deref(), Some("Deutsch"));
        assert_eq!(name_in_catalog(r#"{"_language": "  "}"#), None);
        assert_eq!(name_in_catalog(r#"{"Save": "x"}"#), None);
        let mut c = Catalog::from_json("de", with_name).unwrap();
        assert_eq!(c.translate("Save"), "Speichern");
        assert!(!c.map.contains_key(LANGUAGE_KEY));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "has no value")]
    fn a_placeholder_without_a_value_is_caught_in_debug_builds() {
        fill("a {missing} b", &[]);
    }

    #[test]
    fn english_is_the_identity_and_placeholders_are_filled() {
        assert_eq!(fill("Saved {path}", &[("path", &"/tmp/x")]), "Saved /tmp/x");
        assert_eq!(
            fill("{a} and {b}", &[("b", &2), ("a", &1)]),
            "1 and 2",
            "order does not matter"
        );
        assert_eq!(fill("no braces", &[("x", &1)]), "no braces");
        assert_eq!(fill("odd { brace", &[]), "odd { brace");
        assert_eq!(fill("{n}{n}", &[("n", &7)]), "77");
        assert_eq!(placeholders("{a} then {b} then {a}"), ["a", "b", "a"]);
    }

    #[test]
    fn a_catalog_translates_and_falls_back_to_english() {
        let mut c = Catalog::from_json("de", DE).unwrap();
        assert_eq!(c.translate("Save"), "Speichern");
        assert_eq!(c.translate("Open"), "Open", "no entry: English");
        assert_eq!(
            c.translate("Untranslated"),
            "Untranslated",
            "an empty entry is not a translation"
        );
        assert_eq!(
            c.translate("Weird"),
            "Weird",
            "an entry of the wrong type is ignored"
        );
        assert_eq!(
            fill(c.translate("Saved {path}"), &[("path", &"/x")]),
            "/x gespeichert"
        );
    }

    #[test]
    fn plural_forms_follow_the_language_rule() {
        let c = Catalog::from_json("de", DE).unwrap();
        assert_eq!(c.plural("{n} step", 1), Some("{n} Schritt"));
        assert_eq!(c.plural("{n} step", 0), Some("{n} Schritte"));
        assert_eq!(c.plural("{n} step", 5), Some("{n} Schritte"));
        assert_eq!(c.plural("nothing", 1), None);
        assert_eq!(
            (
                plural_index("fr", 0),
                plural_index("fr", 1),
                plural_index("fr", 2)
            ),
            (0, 0, 1),
            "French: 0 is singular"
        );
        assert_eq!(
            (
                plural_index("en", 0),
                plural_index("en", 1),
                plural_index("en", 2)
            ),
            (1, 0, 1)
        );
        assert_eq!(plural_index("ja", 7), 0, "no plural in Japanese");
    }

    #[test]
    fn a_bad_catalog_is_an_error_not_a_crash() {
        assert!(Catalog::from_json("xx", "not json").is_err());
        assert!(Catalog::from_json("xx", "[1,2]").is_err());
    }

    #[test]
    fn pseudo_text_is_longer_accented_and_keeps_placeholders() {
        let p = pseudo("Saved {path} to the folder");
        assert!(p.starts_with('[') && p.ends_with(']'));
        assert!(p.contains("{path}"), "{p}");
        assert!(p.chars().count() > "Saved {path} to the folder".chars().count());
        assert!(p.contains('é') || p.contains('à'), "{p}");
        assert_eq!(placeholders(&p), ["path"]);
        assert_eq!(pseudo("Save"), pseudo("Save"), "deterministic");
        assert_eq!(
            fill(&pseudo("Hi {name}"), &[("name", &"Bo")])
                .matches("Bo")
                .count(),
            1
        );
    }

    #[test]
    fn language_detection_prefers_explicit_choices_over_the_desktop() {
        // Environment variables are process-wide, so this is the only test that touches them.
        std::env::remove_var("SNAPFORGE_LANG");
        std::env::set_var("LC_ALL", "de_DE.UTF-8");
        assert_eq!(detect(""), "de");
        assert_eq!(detect("fr"), "fr", "a saved choice beats the desktop");
        std::env::set_var("SNAPFORGE_LANG", "pseudo");
        assert_eq!(
            detect("fr"),
            "pseudo",
            "the environment override beats everything"
        );
        std::env::remove_var("SNAPFORGE_LANG");
        std::env::set_var("LC_ALL", "C");
        std::env::remove_var("LC_MESSAGES");
        std::env::remove_var("LANGUAGE");
        std::env::set_var("LANG", "en_US.UTF-8");
        assert_eq!(detect(""), "en");
        std::env::set_var("LANG", "C");
        assert_eq!(detect(""), "", "C means no preference");
    }

    #[test]
    fn the_global_functions_switch_languages() {
        assert!(set_language(""));
        assert_eq!(tr("Anything"), "Anything");
        assert_eq!(tr_n("{n} step", "{n} steps", 1, &[]), "1 step");
        assert_eq!(tr_n("{n} step", "{n} steps", 3, &[]), "3 steps");
        assert!(set_language(PSEUDO));
        assert_ne!(tr("Anything"), "Anything");
        assert!(tr("Anything").contains('ñ'));
        assert_eq!(tr("Anything"), tr("Anything"), "stable");
        assert!(tr_fmt("Saved {path}", &[("path", &"/x")]).contains("/x"));
        assert!(!set_language("zz"), "no such catalog");
        assert_eq!(
            tr("Anything"),
            "Anything",
            "an unknown language falls back to English"
        );
        assert!(set_language("en"));
    }
}
