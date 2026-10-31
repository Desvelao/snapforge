//! Keeps the interface translatable.
//!
//! 1. No user-visible text may bypass `tr` / `tr_fmt` / `tr_n` (heuristic scan of the UI source).
//!    A line that is deliberately not translated (test hooks, identifiers, machine-readable output)
//!    carries the marker `// i18n-ok`.
//! 2. `locales/template.json` lists exactly the messages in the source. Regenerate it with
//!    `UPDATE_I18N=1 cargo test -p guide-app --test i18n_guard`.
//! 3. Every translation file only translates messages that exist and keeps their placeholders.
use std::{collections::BTreeSet, fs, path::PathBuf};

use regex::Regex;

const UI_SOURCES: &[&str] = &[
    "src/app.rs",
    "src/project/mod.rs",
    "src/editor/mod.rs",
    "src/editor/paint.rs",
    "src/editor/tools_ui.rs",
];

/// Lines containing any of these are not user-visible text.
const SKIP_LINE: &[&str] = &[
    "i18n-ok",
    "env::var",
    ".expect(",
    "assert",
    "SNAPFORGE_",
    "Id::new",
    "from_id_salt",
    "panic!",
    "eprintln!",
    "println!",
    "Command::",
    ".env(",
];

const LIT: &str = r#""((?:[^"\\]|\\.)*)""#;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The source up to its unit tests.
fn production_source(rel: &str) -> String {
    let text = fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    match text.find("#[cfg(test)]") {
        Some(i) => text[..i].to_string(),
        None => text,
    }
}

fn unescape(raw: &str) -> String {
    let joined = Regex::new(r"\\\n\s*")
        .unwrap()
        .replace_all(raw, "")
        .into_owned();
    joined
        .replace("\\\"", "\"")
        .replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\\\", "\\")
}

fn line_of(src: &str, byte: usize) -> usize {
    src[..byte].matches('\n').count()
}

fn line_text(src: &str, line: usize) -> &str {
    src.lines().nth(line).unwrap_or("")
}

fn skipped(src: &str, byte: usize) -> bool {
    let line = line_text(src, line_of(src, byte));
    SKIP_LINE.iter().any(|m| line.contains(m))
}

/// The words of a message, with `{placeholders}` removed.
fn words(text: &str) -> usize {
    let outside = Regex::new(r"\{[^}]*\}")
        .unwrap()
        .replace_all(text, " ")
        .into_owned();
    Regex::new(r"[A-Za-z]{3,}")
        .unwrap()
        .find_iter(&outside)
        .count()
}

/// The source with the properly translated literals blanked out.
fn without_translated(src: &str) -> String {
    let calls = Regex::new(&format!(
        r"\btr(?:_fmt)?\(\s*{LIT}|\btr_n\(\s*{LIT}\s*,\s*{LIT}"
    ))
    .unwrap();
    calls
        .replace_all(src, |c: &regex::Captures| " ".repeat(c[0].len()))
        .into_owned()
}

/// Every message the source asks to translate (plural ones are marked `true`).
fn messages() -> BTreeSet<(String, bool)> {
    let one = Regex::new(&format!(r"\btr(?:_fmt)?\(\s*{LIT}")).unwrap();
    let plural = Regex::new(&format!(r"\btr_n\(\s*{LIT}\s*,\s*{LIT}")).unwrap();
    let mut out = BTreeSet::new();
    for rel in UI_SOURCES {
        let src = production_source(rel);
        for c in one.captures_iter(&src) {
            out.insert((unescape(&c[1]), false));
        }
        for c in plural.captures_iter(&src) {
            out.insert((unescape(&c[1]), true));
        }
    }
    out
}

fn placeholders(s: &str) -> BTreeSet<String> {
    Regex::new(r"\{([A-Za-z_][A-Za-z0-9_]*)\}")
        .unwrap()
        .captures_iter(s)
        .map(|c| c[1].to_string())
        .collect()
}

/// Places where a literal or a `format!` goes straight to the screen or into a message field.
fn sink_patterns() -> Vec<Regex> {
    [
        format!(r"\.(?:label|heading|strong|small_button|button|on_hover_text|hint_text|weak|monospace)\(\s*(?:&?format!\(\s*)?{LIT}"),
        format!(r"\.colored_label\([^,;]+,\s*(?:&?format!\(\s*)?{LIT}"),
        format!(r"\.(?:checkbox|toggle_value)\([^,;]+,\s*{LIT}"),
        format!(r"\.selectable_label\([^,;]+,\s*{LIT}"),
        format!(r"\.selectable_value\([^,;]+,[^,;]+,\s*{LIT}"),
        format!(r"(?:Button|CollapsingHeader|Window|Checkbox|RadioButton)::new\(\s*(?:[^,;()]+,\s*)?{LIT}"),
        format!(r"\b(?:status|shortcut_msg)\s*=\s*(?:&?format!\(\s*)?{LIT}"),
        format!(r"\bnotify\(\s*(?:format!\(\s*)?{LIT}"),
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap_or_else(|e| panic!("{p}: {e}")))
    .collect()
}

#[test]
fn no_user_visible_text_bypasses_translation() {
    let sinks = sink_patterns();
    let mut problems = Vec::new();
    for rel in UI_SOURCES {
        let src = production_source(rel);
        for re in &sinks {
            for m in re.captures_iter(&src) {
                let whole = m.get(0).unwrap();
                let lit = m.get(1).map_or("", |g| g.as_str());
                let is_format = whole.as_str().contains("format!");
                // A literal with no letters (units, separators) is not a message, and a format
                // string with no words of its own (numbers, paths) is not one either.
                let is_message = if is_format {
                    words(lit) > 0
                } else {
                    lit.chars().any(char::is_alphabetic)
                };
                if !is_message || skipped(&src, whole.start()) {
                    continue;
                }
                problems.push(format!(
                    "{rel}:{}: {}",
                    line_of(&src, whole.start()) + 1,
                    whole
                        .as_str()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
        }
    }
    problems.sort();
    problems.dedup();
    assert!(
        problems.is_empty(),
        "{} user-visible text(s) are not translatable; wrap them in tr(), tr_fmt() or tr_n():\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// Technical strings that are never shown to users: panel and texture ids, internal state names,
/// file-name patterns and keyboard modifier names. Anything else with a word in it must be translated.
const TECHNICAL: &[&str] = &[
    "editor_gui_bar",
    "guide_save_banner",
    "Ctrl+N",
    "Esc",
    "Tab / Shift+Tab",
    "Space / Enter",
    "Up / Down",
    "Alt+Up / Alt+Down",
    "Enter",
    "Ctrl+D",
    "Delete",
    "Ctrl+S",
    "Ctrl+E",
    "Ctrl+Z / Ctrl+Shift+Z",
    "Ctrl+W",
    "Space",
    "Alt+",
    "Ctrl+",
    "Shift+",
    "align",
    "arrangement",
    "controls",
    "desc_override",
    "done",
    "edit",
    "editor-original",
    "editor-patch",
    "editor-preview",
    "editor_properties",
    "editor_props_bottom",
    "editor_tool_palette",
    "editor_status",
    "editor_tools",
    "error",
    "exports",
    "guide",
    "guide_properties",
    "guide_steps",
    "idle",
    "null",
    "placement",
    "ready",
    "rendering",
    "running",
    "scaling",
    "stale",
    "step-{}",
    "{base}-{n}.{}",
    "{base}.{}",
    "{}-edited",
    "{}-{n}.{}",
    "{}.{}",
];

/// The strict rule: every string literal that contains a word must be inside `tr`, `tr_fmt` or
/// `tr_n`, be the *name* of a placeholder (`("name", &value)`), be machine-readable output (contains
/// an escaped quote), sit on a line marked `// i18n-ok`, or be one of the [`TECHNICAL`] strings.
/// This also catches labels in option tables, `match` arms and `if` branches.
fn strict_problems(label: &str, src: &str) -> Vec<String> {
    let lit = Regex::new(LIT).unwrap();
    let after_is_value = Regex::new(r"^\s*,\s*&").unwrap();
    let rest = without_translated(src);
    let mut problems = Vec::new();
    for m in lit.captures_iter(&rest) {
        let whole = m.get(0).unwrap();
        let text = &m[1];
        let line = line_text(&rest, line_of(&rest, whole.start()));
        let before = &line[..line.find(whole.as_str()).unwrap_or(0)];
        if SKIP_LINE.iter().any(|s| line.contains(s))
            || before.contains("//")
            || text.contains("\\\"")
            || text.contains("{{")
            || words(text) == 0
            || TECHNICAL.contains(&text)
            || after_is_value.is_match(&rest[whole.end()..])
        {
            continue;
        }
        problems.push(format!(
            "{label}:{}: \"{text}\"",
            line_of(&rest, whole.start()) + 1
        ));
    }
    problems.sort();
    problems.dedup();
    problems
}

#[test]
fn every_word_bearing_literal_is_translated_or_known_to_be_technical() {
    let problems: Vec<String> = UI_SOURCES
        .iter()
        .flat_map(|rel| strict_problems(rel, &production_source(rel)))
        .collect();
    assert!(
        problems.is_empty(),
        "{} string literal(s) with words are neither translated nor listed as technical.\nWrap user-visible text in tr()/tr_fmt()/tr_n(); add genuinely technical strings to TECHNICAL (or mark the line `// i18n-ok`):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// The strict rule must catch the kinds of text that earlier heuristics let through.
#[test]
fn the_strict_rule_catches_lowercase_labels_option_tables_and_branches() {
    for (bad, what) in [
        (
            r#"flags.push("excluded");"#,
            "a lowercase word pushed into a list",
        ),
        (
            r#"let t = if x { "unsaved changes" } else { "saved" };"#,
            "an if/else branch",
        ),
        (
            r#"combo(ui, "a", &mut v, &[(Align::Left, "left")]);"#,
            "an option table",
        ),
        (r#"Tool::Select => "Select","#, "a match arm"),
        (r#"title = "(untitled)""#, "text starting with a bracket"),
        (
            r#"foo.ok_or("that step no longer exists")?"#,
            "an error message",
        ),
    ] {
        assert!(
            !strict_problems("x", bad).is_empty(),
            "should be flagged ({what}): {bad}"
        );
    }
    for (fine, what) in [
        (r#"flags.push(tr("excluded"));"#, "translated"),
        (
            r#"let a = tr_fmt("Hi {name}", &[("name", &n)]);"#,
            "placeholder name",
        ),
        (
            r#"let a = tr_fmt("Hi {name}", &[("name",
            &n)]);"#,
            "placeholder name, value on the next line",
        ),
        (r#"let k = "Ctrl+";"#, "technical"),
        (r#"let id = "guide_steps"; // a panel id"#, "technical"),
        (
            r#"let x = "Something odd"; // i18n-ok"#,
            "explicitly marked",
        ),
        (
            r#"let j = format!("{{\"a\":{}}}", 1);"#,
            "machine-readable output",
        ),
        (r#"let n = "{}%";"#, "no words"),
    ] {
        assert!(
            strict_problems("x", fine).is_empty(),
            "should be accepted ({what}): {fine}\n{:?}",
            strict_problems("x", fine)
        );
    }
}

#[test]
fn the_technical_list_has_no_stale_entries() {
    // An entry nobody uses any more hides future mistakes: every one must still occur in the source.
    let all: String = UI_SOURCES.iter().map(|r| production_source(r)).collect();
    for t in TECHNICAL {
        assert!(
            all.contains(&format!("\"{t}\"")),
            "TECHNICAL lists \"{t}\" but the source no longer uses it"
        );
    }
}

#[test]
fn the_template_lists_exactly_the_messages_in_the_source() {
    let path = root().join("locales/template.json");
    let found = messages();
    let mut object = serde_json::Map::new();
    for (msgid, plural) in &found {
        object.insert(
            msgid.clone(),
            if *plural {
                serde_json::json!(["", ""])
            } else {
                serde_json::json!("")
            },
        );
    }
    if std::env::var_os("UPDATE_I18N").is_some() {
        fs::write(
            &path,
            serde_json::to_string_pretty(&serde_json::Value::Object(object)).unwrap() + "\n",
        )
        .unwrap();
        return;
    }
    let current: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap_or_else(|_| "{}".into())).unwrap();
    let listed: BTreeSet<String> = current.as_object().unwrap().keys().cloned().collect();
    let wanted: BTreeSet<String> = found.iter().map(|(m, _)| m.clone()).collect();
    let missing: Vec<_> = wanted.difference(&listed).collect();
    let extra: Vec<_> = listed.difference(&wanted).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "locales/template.json is out of date (run `UPDATE_I18N=1 cargo test -p guide-app --test i18n_guard`).\nmissing: {missing:?}\nno longer used: {extra:?}"
    );
}

#[test]
fn translation_files_only_translate_real_messages_and_keep_their_placeholders() {
    let template: BTreeSet<String> = {
        let v: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(root().join("locales/template.json")).unwrap(),
        )
        .unwrap();
        v.as_object().unwrap().keys().cloned().collect()
    };
    let mut checked = 0;
    for entry in fs::read_dir(root().join("locales")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name == "template.json" || !name.ends_with(".json") {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        for (msgid, tr) in v
            .as_object()
            .unwrap_or_else(|| panic!("{name}: not an object"))
        {
            if msgid == "_language" {
                // The language's name in itself, shown in the language picker.
                assert!(
                    tr.as_str().is_some_and(|n| !n.trim().is_empty()),
                    "{name}: \"_language\" must name the language, e.g. \"Deutsch\""
                );
                continue;
            }
            assert!(
                template.contains(msgid),
                "{name}: \"{msgid}\" is not a message in the source"
            );
            let forms: Vec<&str> = match tr {
                serde_json::Value::String(s) => vec![s.as_str()],
                serde_json::Value::Array(a) => a.iter().filter_map(|f| f.as_str()).collect(),
                _ => panic!("{name}: \"{msgid}\" must be a string or a list of strings"),
            };
            for f in forms.into_iter().filter(|f| !f.is_empty()) {
                assert_eq!(
                    placeholders(f),
                    placeholders(msgid),
                    "{name}: \"{msgid}\" -> \"{f}\" changes the placeholders"
                );
            }
        }
        checked += 1;
    }
    eprintln!("checked {checked} translation file(s)");
}

/// Guards the guard: these snippets must be flagged, and these must be accepted.
#[test]
fn the_scanner_recognises_the_patterns_it_is_meant_to_catch() {
    let sinks = sink_patterns();
    let flagged = |s: &str| {
        sinks.iter().any(|re| {
            re.captures_iter(s).any(|m| {
                let lit = m.get(1).map_or("", |g| g.as_str());
                if m.get(0).unwrap().as_str().contains("format!") {
                    words(lit) > 0
                } else {
                    lit.chars().any(char::is_alphabetic)
                }
            })
        })
    };
    for bad in [
        r#"ui.label("Hello");"#,
        "ui.button(\n    \"Save\",\n)",
        r#"ui.checkbox(&mut x, "Include cursor")"#,
        r#"self.status = format!("Failed: {e}");"#,
        r#"self.status = "Done".into();"#,
        r#"g.notify(format!("Cannot edit: {e}"))"#,
        r#"ui.selectable_value(&mut a, B, "Label")"#,
        r#"egui::Button::new("Open")"#,
        "ui.label(format!(\n    \"Step {n} of {total}\",\n))",
    ] {
        assert!(flagged(bad), "should be flagged: {bad}");
    }
    for fine in [
        r#"ui.label(tr("Hello"));"#,
        r#"ui.label(tr_fmt("Hi {x}", &[("x", &1)]))"#,
        r#"ui.label(&self.status)"#,
        r#"self.status = tr("Done").into();"#,
        r#"ui.label(format!("{:.0}%", z))"#,
        r#"ui.label(format!("{} ({} x {})", a, b, c))"#,
        r#"ui.label("-")"#,
    ] {
        assert!(!flagged(fine), "should be accepted: {fine}");
    }
    assert_eq!(words("{a} of {b}"), 0);
    assert_eq!(words("Step {n} of {total}"), 1);
    assert_eq!(placeholders("{a} then {b} then {a}").len(), 2);
}
