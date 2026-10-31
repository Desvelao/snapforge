//! Format regression test: `tests/fixtures/project-guide.<ext>` and `project-plain.<ext>` (`<ext>` is `project_extension` in `app.toml`) are
//! hand-written projects in format version 1. It is committed once and must never be regenerated: every future build has
//! to keep opening it with the same content. If this test fails after a change, the change
//! broke existing users' files - add a migration (see docs/project-format.md) instead of
//! editing the fixture.
use std::{
    fs,
    path::{Path, PathBuf},
};

use guide_core::{ItemId, Kind, Layout, ObscureMode, ProjectKind};
use guide_storage::ProjectStore;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.{}", guide_core::PROJECT_EXTENSION))
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.path().is_dir() {
            copy_tree(&e.path(), &to.join(e.file_name()));
        } else {
            fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

/// A private copy, so the committed fixture is never modified by a test.
fn copy_of_fixture(name: &str, tag: &str) -> PathBuf {
    let dst = std::env::temp_dir().join(format!(
        "snapforge-golden-{name}-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dst);
    copy_tree(&fixture(name), &dst);
    dst
}

#[test]
fn the_version_1_guide_fixture_opens_cleanly_with_all_its_content() {
    let d = copy_of_fixture("project-guide", "open");
    let o = ProjectStore::open(&d).unwrap();
    assert_eq!(
        o.notice, None,
        "a current-format file needs no upgrade or repair"
    );
    assert!(o.issues.is_empty(), "{:?}", o.issues);

    let p = &o.project;
    assert_eq!(p.schema_version, 1);
    assert_eq!(p.project_id, "p-0123456789abcdef0123456789abcdef");
    let g = p.guide().expect("this fixture is a guide");
    assert_eq!(
        (p.name.as_str(), g.final_notes.as_str()),
        ("Golden guide", "The end.")
    );
    assert!(g.introduction.starts_with("This folder pins version 1"));
    assert_eq!(
        p.items.iter().map(|s| s.id).collect::<Vec<_>>(),
        [ItemId(1), ItemId(2)]
    );

    let (one, two) = (&p.items[0], &p.items[1]);
    assert_eq!(
        (one.title.as_str(), one.description.as_str(), one.included),
        ("Open the menu", "Click the menu button.", true)
    );
    assert_eq!(one.caption, "The menu button");
    assert_eq!(one.image, "assets/shot-golden-1.png");
    let kinds: Vec<&str> = one
        .annotations
        .annotations
        .iter()
        .map(|a| match &a.kind {
            Kind::Rectangle { .. } => "Rectangle",
            Kind::Marker { .. } => "Marker",
            Kind::Text { .. } => "Text",
            Kind::Obscure {
                mode: ObscureMode::Pixelate,
                strength: 2,
                ..
            } => "Obscure",
            Kind::Arrow { .. } => "Arrow",
            Kind::Pen { points } if points.len() == 2 => "Pen",
            Kind::Highlighter { .. } => "Highlighter",
            Kind::Line { .. } => "Line",
            Kind::Ellipse { .. } => "Ellipse",
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "Rectangle",
            "Marker",
            "Text",
            "Obscure",
            "Arrow",
            "Pen",
            "Highlighter",
            "Line",
            "Ellipse"
        ],
        "every annotation type of format 1"
    );
    assert!(
        one.annotations
            .annotations
            .iter()
            .any(|a| a.style.fill.is_some()),
        "a filled shape"
    );
    assert!(
        matches!(&one.annotations.annotations[2].kind, Kind::Text { text, bold: true, .. } if text == "Hi")
    );

    assert!(!two.included);
    assert_eq!(two.annotations.crop.map(|c| (c.w, c.h)), Some((6, 4)));
    let layout = p.step_layout(two.id);
    assert_eq!(layout.and_then(|l| l.max_width), Some(300));
    assert_eq!(layout.and_then(|l| l.show_description), Some(false));

    let c = &g.composition;
    assert_eq!((c.margin, c.spacing, c.max_width), (32, 24, 1200));
    assert_eq!(c.layout, Layout::Vertical);
    assert!(c.show_numbers && c.show_titles && c.show_descriptions && c.show_final_notes);
    assert_eq!(c.font_size, 20.0);

    // The screenshots are readable and the annotations fit them.
    assert_eq!(o.store.load_asset(&one.image).unwrap().width, 8);
    assert_eq!(
        o.store.item_sizes(p).get(&ItemId(2)),
        Some(&(6, 4)),
        "the crop decides the finished size"
    );
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn the_version_1_plain_fixture_opens_cleanly_with_all_its_content() {
    let d = copy_of_fixture("project-plain", "open");
    let o = ProjectStore::open(&d).unwrap();
    assert_eq!(o.notice, None);
    assert!(o.issues.is_empty(), "{:?}", o.issues);

    let p = &o.project;
    assert_eq!(p.schema_version, 1);
    assert_eq!(p.project_id, "p-fedcba9876543210fedcba9876543210");
    assert_eq!(p.name, "Golden plain project");
    assert_eq!(p.kind, ProjectKind::Plain);
    assert!(p.guide().is_none());

    let (before, after) = (&p.items[0], &p.items[1]);
    assert_eq!(
        (
            before.title.as_str(),
            before.description.as_str(),
            before.caption.as_str()
        ),
        ("Before", "The window as it is.", "Before the change")
    );
    assert_eq!(before.annotations.annotations.len(), 1);
    assert!(!after.included);
    assert_eq!(after.annotations.crop.map(|c| (c.w, c.h)), Some((6, 4)));
    assert_eq!(o.store.item_sizes(p).get(&ItemId(2)), Some(&(6, 4)));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn opening_and_saving_a_fixture_changes_nothing_that_matters() {
    for name in ["project-guide", "project-plain"] {
        let d = copy_of_fixture(name, "resave");
        let first = ProjectStore::open(&d).unwrap();
        first.store.save(&first.project).unwrap();
        let second = ProjectStore::open(&d).unwrap();
        assert_eq!(
            second.project, first.project,
            "{name}: a save/open cycle keeps every value"
        );
        let read = |p: &Path| -> serde_json::Value {
            serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
        };
        assert_eq!(
            read(&d.join("project.json")),
            read(&fixture(name).join("project.json")),
            "{name}: the written file is the same JSON document as the committed one"
        );
        fs::remove_dir_all(&d).unwrap();
    }
}

#[test]
fn the_guide_fixture_can_still_be_laid_out() {
    let d = copy_of_fixture("project-guide", "layout");
    let o = ProjectStore::open(&d).unwrap();
    let sizes = o.store.item_sizes(&o.project);
    let layout = guide_render::compose::compute_layout(&o.project, &sizes).unwrap();
    assert_eq!(layout.boxes.len(), 1, "only the included step is laid out");
    assert_eq!(layout.width, 1200 + 2 * 32);
    fs::remove_dir_all(&d).unwrap();
}
