//! `snapforge --check <folder>`: validates a project (a guide or a plain project) and prints what it finds,
//! without opening a window. Exit codes: 0 = fine, 1 = cannot be opened, 2 = opens but has problems.

use std::{fmt::Write, path::Path};

use guide_render::compose::compute_layout;
use guide_storage::ProjectStore;

pub const OK: i32 = 0;
pub const CANNOT_OPEN: i32 = 1;
pub const PROBLEMS: i32 = 2;

/// Returns the exit code and the text to print.
pub fn run(dir: &Path) -> (i32, String) {
    let o = match ProjectStore::open(dir) {
        Ok(o) => o,
        Err(e) => return (CANNOT_OPEN, format!("Cannot open {}: {e}\n", dir.display())),
    };
    let mut out = String::new();
    let mut problems = 0;
    let p = &o.project;
    let included = p.included_items().count();
    let (kind, unit) = if p.is_guide() {
        ("Guide", "step(s)")
    } else {
        ("Project", "screenshot(s)")
    };
    let _ = writeln!(out, "{kind} \"{}\" (project {})", p.name, p.project_id);
    let _ = writeln!(
        out,
        "Format version {}; {} {unit}, {included} included in exports",
        p.schema_version,
        p.items.len()
    );
    if let Some(n) = &o.notice {
        problems += 1;
        let _ = writeln!(out, "Notice: {n}");
    }
    for issue in &o.issues {
        problems += 1;
        let _ = writeln!(out, "Problem: {}", issue.message);
    }
    if p.is_guide() {
        match compute_layout(p, &o.store.item_sizes(p)) {
            Ok(l) => {
                let _ = writeln!(out, "Combined image would be {} x {} px", l.width, l.height);
            }
            Err(e) => {
                problems += 1;
                let _ = writeln!(out, "Problem: {e}");
            }
        }
    }
    let _ = writeln!(
        out,
        "{}",
        if problems == 0 {
            "OK".to_string()
        } else {
            format!("{problems} problem(s) found")
        }
    );
    (if problems == 0 { OK } else { PROBLEMS }, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use guide_core::{NewItem, Project};
    use std::fs;

    fn project(tag: &str) -> (ProjectStore, Project, std::path::PathBuf) {
        project_of(tag, Project::new_guide("Checked guide"))
    }

    fn project_of(tag: &str, mut g: Project) -> (ProjectStore, Project, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!("snapforge-check-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let p = ProjectStore::create(&d, &g).unwrap();
        for i in 0..2u8 {
            let rel = p
                .add_asset_rgba(8, 8, &[i * 50, 0, 0, 255].repeat(64))
                .unwrap();
            g.add_item(
                usize::MAX,
                NewItem {
                    title: format!("S{i}"),
                    description: String::new(),
                    caption: String::new(),
                    image: rel,
                },
            );
        }
        p.save(&g).unwrap();
        (p, g, d)
    }

    #[test]
    fn a_healthy_project_is_ok() {
        let (_, _, d) = project("ok");
        let (code, text) = run(&d);
        assert_eq!(code, OK, "{text}");
        assert!(
            text.contains("Checked guide")
                && text.contains("2 step(s)")
                && text.contains("Combined image would be")
                && text.trim_end().ends_with("OK"),
            "{text}"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_plain_project_is_checked_without_a_combined_image() {
        let (_, _, d) = project_of("plain", Project::new_plain("Some screenshots"));
        let (code, text) = run(&d);
        assert_eq!(code, OK, "{text}");
        assert!(
            text.contains("Project \"Some screenshots\"")
                && text.contains("2 screenshot(s)")
                && !text.contains("Combined image")
                && text.trim_end().ends_with("OK"),
            "{text}"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn missing_screenshots_are_listed_with_the_exit_code_for_problems() {
        let (_, g, d) = project("missing");
        fs::remove_file(d.join(&g.items[1].image)).unwrap();
        let (code, text) = run(&d);
        assert_eq!(code, PROBLEMS, "{text}");
        assert!(
            text.contains("Problem:")
                && text.contains("S1")
                && text.contains("missing")
                && text.contains("1 problem(s) found"),
            "{text}"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_project_that_cannot_be_opened_says_why() {
        let (_, _, d) = project("broken");
        fs::remove_file(d.join("project.json.bak")).ok();
        fs::write(d.join("project.json"), "{ nope").unwrap();
        let (code, text) = run(&d);
        assert_eq!(code, CANNOT_OPEN);
        assert!(
            text.contains("project.json") && text.contains("damaged"),
            "{text}"
        );
        let (code, text) = run(&d.join("not-a-project"));
        assert_eq!(code, CANNOT_OPEN);
        assert!(text.contains("not a project"), "{text}");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_repaired_project_reports_what_was_repaired() {
        let (p, mut g, d) = project("repaired");
        g.name = "Second".into();
        p.save(&g).unwrap();
        fs::write(d.join("project.json"), "garbage").unwrap();
        let (code, text) = run(&d);
        assert_eq!(code, PROBLEMS, "recovery is worth a warning: {text}");
        assert!(
            text.contains("Notice:") && text.contains("project.json.bak"),
            "{text}"
        );
        fs::remove_dir_all(&d).unwrap();
    }
}
