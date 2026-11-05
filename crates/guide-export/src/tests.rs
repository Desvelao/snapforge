use std::{
    fs,
    path::{Path, PathBuf},
};

use guide_core::{Editor, ImageScaling, Kind, NewItem, PixelRect, Project, RectF, Style};

use super::*;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("guide-export-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    d
}

fn solid(w: u32, h: u32, c: [u8; 3]) -> Vec<u8> {
    [c[0], c[1], c[2], 255].repeat(w as usize * h as usize)
}

const COLOURS: [[u8; 3]; 5] = [
    [200, 40, 40],
    [40, 200, 40],
    [40, 40, 200],
    [200, 200, 40],
    [40, 200, 200],
];

struct Fixture {
    store: ProjectStore,
    project: Project,
    root: PathBuf,
}

/// A guide with the given step titles; step `i` is a solid-coloured 320x200 screenshot.
fn fixture(tag: &str, titles: &[&str]) -> Fixture {
    let root = dir(tag);
    let mut project = Project::new_guide("Change a setting");
    let store = ProjectStore::create(
        &root.join(format!("g.{}", guide_core::PROJECT_EXTENSION)),
        &project,
    )
    .unwrap();
    for (i, t) in titles.iter().enumerate() {
        let rel = store
            .add_asset_rgba(320, 200, &solid(320, 200, COLOURS[i % 5]))
            .unwrap();
        project.add_item(
            usize::MAX,
            NewItem {
                title: (*t).into(),
                description: format!("Details for {t}"),
                caption: String::new(),
                image: rel,
            },
        );
    }
    store.save(&project).unwrap();
    Fixture {
        store,
        project,
        root,
    }
}

fn opts(f: &Fixture, mode: ExportMode) -> ExportOptions {
    ExportOptions {
        dir: f.root.join("out"),
        format: ImageFormat::Png,
        mode,
        names: NameStyle::NumberAndTitle,
        include_text: false,
        replace_existing: false,
        missing_label: None,
    }
}

fn run(f: &Fixture, o: &ExportOptions) -> ExportReport {
    export(&f.store, &f.project, o, &Cancel::default(), &mut |_| {})
}

fn names(r: &ExportReport) -> Vec<String> {
    r.written
        .iter()
        .map(|w| w.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

fn px(path: &Path, x: u32, y: u32) -> [u8; 3] {
    let p = image::open(path).unwrap().to_rgba8().get_pixel(x, y).0;
    [p[0], p[1], p[2]]
}

fn all_files(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(d)
        .map(|r| {
            r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn file_names_are_numbered_sorted_and_safe() {
    let n = NameStyle::NumberAndTitle;
    assert_eq!(file_stem(1, 3, "Open settings", n), "01-open-settings");
    assert_eq!(file_stem(12, 12, "Save & close!", n), "12-save-close");
    assert_eq!(
        file_stem(7, 120, "x", n),
        "007-x",
        "wider numbers once there are 100 or more steps"
    );
    assert_eq!(file_stem(1, 3, "", n), "01-step");
    assert_eq!(
        file_stem(1, 3, "日本語", n),
        "01-step",
        "nothing portable left of the title"
    );
    assert_eq!(file_stem(2, 9, "Any title", NameStyle::NumberOnly), "02");
    assert_eq!(
        file_stem(7, 120, "Any title", NameStyle::NumberOnly),
        "007",
        "the number is padded the same way without a title"
    );
    for evil in [
        "../../etc/passwd",
        "a/b\\c",
        "con.",
        ".hidden",
        "x\0y",
        "COM1:",
        "  ",
    ] {
        let s = file_stem(1, 3, evil, n);
        assert!(
            s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{evil:?} -> {s:?}"
        );
        assert!(s.starts_with("01-"));
    }
    assert!(file_stem(1, 3, &"long title ".repeat(50), n).len() <= 3 + 50);
    // Sorting the names sorts the steps.
    let mut stems: Vec<String> = (1..=12).map(|i| file_stem(i, 12, "t", n)).collect();
    let before = stems.clone();
    stems.sort();
    assert_eq!(stems, before);
    let g = Project::new_guide("My Great Guide!");
    assert_eq!(combined_stem(&g), "my-great-guide");
    assert_eq!(combined_stem(&Project::new_guide("???")), "guide");
}

#[test]
fn the_plan_lists_exactly_the_files_the_export_writes() {
    let mut f = fixture("plan", &["Open settings", "Skip me", "Save"]);
    let skipped = f.project.items[1].id;
    f.project.set_included(skipped, false);
    for mode in [ExportMode::Separate, ExportMode::Combined] {
        for format in [ImageFormat::Png, ImageFormat::Jpeg] {
            let mut o = opts(&f, mode);
            o.format = format;
            o.dir = f.root.join(format!("plan-{mode:?}-{format:?}"));
            let plan = planned_files(&f.project, &o);
            let r = run(&f, &o);
            assert_eq!(plan, names(&r), "{mode:?} {format:?}");
        }
    }
}

#[test]
fn separate_export_writes_included_steps_in_order_with_predictable_names() {
    let mut f = fixture(
        "separate",
        &[
            "Open settings",
            "Pick option",
            "Save changes",
            "Close window",
        ],
    );
    let skipped = f.project.items[1].id;
    f.project.set_included(skipped, false);
    let r = run(&f, &opts(&f, ExportMode::Separate));
    assert!(r.is_complete(), "{:?}", r.problems);
    assert_eq!(
        names(&r),
        [
            "01-open-settings.png",
            "02-save-changes.png",
            "03-close-window.png"
        ],
        "numbered by position among the included steps"
    );
    assert_eq!(
        all_files(&f.root.join("out")),
        names(&r),
        "nothing else was written"
    );
    // Each file is the right step's screenshot, in order.
    for (w, colour) in r.written.iter().zip([COLOURS[0], COLOURS[2], COLOURS[3]]) {
        assert_eq!((w.width, w.height), (320, 200));
        assert_eq!(px(&w.path, 160, 100), colour);
    }
    assert_eq!(
        r.written.iter().map(|w| w.step).collect::<Vec<_>>(),
        [
            Some(f.project.items[0].id),
            Some(f.project.items[2].id),
            Some(f.project.items[3].id)
        ]
    );
}

#[test]
fn separate_images_have_annotations_burned_in_and_crops_applied() {
    let mut f = fixture("annotated", &["One", "Two"]);
    let mut e = Editor::new(guide_core::Document::default());
    e.add(
        Kind::Rectangle {
            rect: RectF {
                x: 20.0,
                y: 20.0,
                w: 100.0,
                h: 60.0,
            },
        },
        Style {
            color: guide_core::Rgba([255, 255, 255, 255]),
            width: 6.0,
            fill: None,
        },
    );
    f.project.items[0].annotations = e.document().clone();
    let mut c = Editor::new(guide_core::Document::default());
    c.set_crop(Some(PixelRect {
        x: 10,
        y: 10,
        w: 100,
        h: 50,
    }));
    f.project.items[1].annotations = c.document().clone();
    let r = run(&f, &opts(&f, ExportMode::Separate));
    assert!(r.is_complete(), "{:?}", r.problems);
    assert_eq!(
        px(&r.written[0].path, 20, 50),
        [255, 255, 255],
        "the rectangle outline is in the exported file"
    );
    assert_eq!(
        px(&r.written[0].path, 200, 150),
        COLOURS[0],
        "away from it the screenshot is unchanged"
    );
    assert_eq!(
        (r.written[1].width, r.written[1].height),
        (100, 50),
        "the crop decides the exported size"
    );
}

#[test]
fn titles_and_descriptions_can_be_drawn_into_each_separate_image() {
    let f = fixture("withtext", &["Open settings", "Save"]);
    let mut o = opts(&f, ExportMode::Separate);
    let plain = run(&f, &o);
    o.dir = f.root.join("out-text");
    o.include_text = true;
    let texted = run(&f, &o);
    assert!(texted.is_complete(), "{:?}", texted.problems);
    for (a, b) in plain.written.iter().zip(&texted.written) {
        assert!(
            b.height > a.height + 30,
            "room for the title and description: {} vs {}",
            b.height,
            a.height
        );
        assert!(b.width >= a.width);
    }
    assert_eq!(
        names(&plain),
        names(&texted),
        "the file names do not change"
    );
    // The screenshot itself is still in there, undistorted, at its natural size.
    let img = image::open(&texted.written[0].path).unwrap().to_rgba8();
    let n = img.pixels().filter(|p| p.0[..3] == COLOURS[0]).count();
    assert_eq!(
        n,
        320 * 200,
        "every screenshot pixel is present exactly once"
    );
}

#[test]
fn every_format_produces_a_readable_file_of_the_right_size() {
    let f = fixture("formats", &["A"]);
    for (format, ext) in [
        (ImageFormat::Png, "png"),
        (ImageFormat::Jpeg, "jpg"),
        (ImageFormat::WebP, "webp"),
    ] {
        let mut o = opts(&f, ExportMode::Separate);
        o.format = format;
        o.dir = f.root.join(format!("out-{ext}"));
        let r = run(&f, &o);
        assert!(r.is_complete(), "{ext}: {:?}", r.problems);
        assert_eq!(names(&r), [format!("01-a.{ext}")]);
        assert_eq!(
            image::image_dimensions(&r.written[0].path).unwrap(),
            (320, 200),
            "{ext}"
        );
    }
}

#[test]
fn combined_export_is_one_image_with_every_step_in_order() {
    let f = fixture("combined", &["First", "Second", "Third"]);
    let r = run(&f, &opts(&f, ExportMode::Combined));
    assert!(r.is_complete(), "{:?}", r.problems);
    assert_eq!(names(&r), ["change-a-setting.png"]);
    let sizes = f.store.item_sizes(&f.project);
    let layout = compute_layout(&f.project, &sizes).unwrap();
    assert_eq!(
        (r.written[0].width, r.written[0].height),
        (layout.width, layout.height),
        "exactly the size the layout engine predicted"
    );
    let mut last_y = -1.0;
    for (item, colour) in layout.images.iter().zip(COLOURS) {
        assert!(
            item.rect.y > last_y,
            "steps run top to bottom in guide order"
        );
        last_y = item.rect.y;
        assert_eq!(
            px(
                &r.written[0].path,
                (item.rect.x + item.rect.w / 2.0) as u32,
                (item.rect.y + item.rect.h / 2.0) as u32
            ),
            colour
        );
    }
    assert_eq!(all_files(&f.root.join("out")).len(), 1);
}

#[test]
fn existing_files_are_kept_unless_replacing_is_asked_for() {
    let f = fixture("overwrite", &["One", "Two"]);
    let o = opts(&f, ExportMode::Separate);
    let first = run(&f, &o);
    let before: Vec<Vec<u8>> = first
        .written
        .iter()
        .map(|w| fs::read(&w.path).unwrap())
        .collect();
    let second = run(&f, &o);
    assert_eq!(
        names(&second),
        ["01-one-2.png", "02-two-2.png"],
        "never overwrites: the new files get -2"
    );
    for (w, bytes) in first.written.iter().zip(&before) {
        assert!(
            fs::read(&w.path).unwrap() == *bytes,
            "the earlier export is untouched"
        );
    }

    // Replace: same names, new content, no leftovers.
    let mut g = f.project.clone();
    let mut e = Editor::new(guide_core::Document::default());
    e.set_crop(Some(PixelRect {
        x: 0,
        y: 0,
        w: 50,
        h: 40,
    }));
    g.items[0].annotations = e.document().clone();
    let mut r = opts(&f, ExportMode::Separate);
    r.dir = f.root.join("out-replace");
    let a = export(&f.store, &f.project, &r, &Cancel::default(), &mut |_| {});
    r.replace_existing = true;
    let b = export(&f.store, &g, &r, &Cancel::default(), &mut |_| {});
    assert_eq!(
        names(&a),
        names(&b),
        "replacing keeps the predictable names"
    );
    assert_eq!((b.written[0].width, b.written[0].height), (50, 40));
    assert_eq!(
        image::image_dimensions(&b.written[0].path).unwrap(),
        (50, 40),
        "the file really was replaced"
    );
    assert_eq!(
        all_files(&f.root.join("out-replace")),
        names(&b),
        "no temp files or duplicates"
    );
}

#[test]
fn a_step_that_cannot_be_exported_is_reported_and_the_others_still_are() {
    let f = fixture("missing", &["Good one", "Broken one", "Good two"]);
    fs::remove_file(f.store.dir().join(&f.project.items[1].image)).unwrap();
    let r = run(&f, &opts(&f, ExportMode::Separate));
    assert!(!r.is_complete());
    assert_eq!(
        names(&r),
        ["01-good-one.png", "03-good-two.png"],
        "the numbers still match the guide's order"
    );
    assert_eq!(r.problems.len(), 1);
    assert_eq!(r.problems[0].step, Some(f.project.items[1].id));
    assert!(
        r.problems[0].message.contains("Broken one")
            && r.problems[0].message.contains("not exported"),
        "{}",
        r.problems[0].message
    );

    // In the combined image a missing screenshot becomes a placeholder, and that is reported too.
    let r = run(&f, &opts(&f, ExportMode::Combined));
    assert_eq!(r.written.len(), 1, "the combined image is still written");
    assert!(
        r.problems
            .iter()
            .any(|p| p.message.contains("placeholder") && p.message.contains("Broken one")),
        "{:?}",
        r.problems
    );
}

#[test]
fn an_unwritable_output_folder_is_reported_and_nothing_is_lost() {
    let f = fixture("badout", &["A"]);
    fs::create_dir_all(&f.root).unwrap();
    fs::write(f.root.join("out"), "I am a file, not a folder").unwrap();
    let r = run(&f, &opts(&f, ExportMode::Separate));
    assert!(r.written.is_empty());
    assert!(
        r.problems[0].message.contains("output folder"),
        "{}",
        r.problems[0].message
    );
    assert_eq!(
        fs::read_to_string(f.root.join("out")).unwrap(),
        "I am a file, not a folder"
    );
    // A folder where one output file should go: that file fails, with its name in the message.
    let mut o = opts(&f, ExportMode::Separate);
    o.dir = f.root.join("out2");
    fs::create_dir_all(o.dir.join("01-a.png")).unwrap();
    o.replace_existing = true;
    let r = run(&f, &o);
    assert!(
        r.written.is_empty() && r.problems[0].message.contains("01-a.png"),
        "{:?}",
        r.problems
    );
    assert!(all_files(&o.dir).iter().all(|n| !n.ends_with(".tmp")));
}

#[test]
fn nothing_to_export_is_an_error_not_an_empty_success() {
    let mut f = fixture("none", &["A", "B"]);
    for s in f.project.items.clone() {
        f.project.set_included(s.id, false);
    }
    for mode in [ExportMode::Separate, ExportMode::Combined] {
        let r = run(&f, &opts(&f, mode));
        assert!(r.written.is_empty() && !r.is_complete());
        assert!(
            r.problems[0].message.contains("No steps are included"),
            "{}",
            r.problems[0].message
        );
    }
    assert!(
        !f.root.join("out").exists(),
        "no folder was created for nothing"
    );
}

/// A PNG header that claims a huge size, so the size limit can be tested without huge files.
fn fake_png(width: u32, height: u32) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for b in data {
            crc ^= u32::from(*b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(13u32.to_be_bytes());
    png.extend(&ihdr);
    png.extend(crc32(&ihdr).to_be_bytes());
    png.extend(0u32.to_be_bytes());
    png.extend(b"IDAT");
    png.extend(crc32(b"IDAT").to_be_bytes());
    png
}

#[test]
fn an_oversized_combined_image_is_refused_up_front_with_alternatives() {
    let root = dir("huge");
    let mut project = Project::new_guide("Huge");
    let store = ProjectStore::create(
        &root.join(format!("g.{}", guide_core::PROJECT_EXTENSION)),
        &project,
    )
    .unwrap();
    project.guide_mut().unwrap().composition.max_width = 10_000;
    project.guide_mut().unwrap().composition.image_scaling = ImageScaling::FillWidth;
    for i in 0..60 {
        let rel = format!("assets/shot-huge-{i}.png");
        fs::write(store.dir().join(&rel), fake_png(4000, 4000)).unwrap();
        project.add_item(
            usize::MAX,
            NewItem {
                title: format!("S{i}"),
                description: String::new(),
                caption: String::new(),
                image: rel,
            },
        );
    }
    store.save(&project).unwrap();
    let o = ExportOptions {
        dir: root.join("out"),
        format: ImageFormat::Png,
        mode: ExportMode::Combined,
        names: NameStyle::default(),
        include_text: false,
        replace_existing: false,
        missing_label: None,
    };
    let r = export(&store, &project, &o, &Cancel::default(), &mut |_| {});
    assert!(r.written.is_empty());
    let m = &r.problems[0].message;
    assert!(
        m.contains("pixel limit") && m.contains("separate images") && m.contains("narrower"),
        "{m}"
    );
    assert!(
        all_files(&root.join("out")).is_empty(),
        "nothing partial was written"
    );
}

#[test]
fn cancelling_stops_cleanly_and_keeps_what_was_already_written() {
    let f = fixture("cancel", &["A", "B", "C"]);
    // Cancelled before it starts.
    let c = Cancel::default();
    c.cancel();
    let r = export(
        &f.store,
        &f.project,
        &opts(&f, ExportMode::Separate),
        &c,
        &mut |_| {},
    );
    assert!(r.cancelled && r.written.is_empty() && !r.is_complete());
    // Cancelled after the first file.
    let c = Cancel::default();
    let c2 = c.clone();
    let mut o = opts(&f, ExportMode::Separate);
    o.dir = f.root.join("out-half");
    let r = export(&f.store, &f.project, &o, &c, &mut |p| {
        if p.done == 1 {
            c2.cancel();
        }
    });
    assert!(r.cancelled);
    assert_eq!(
        names(&r),
        ["01-a.png"],
        "what was done stays, the rest is not started"
    );
    assert_eq!(all_files(&o.dir), ["01-a.png"], "no partial or temp files");
    // Cancelling the combined image writes nothing.
    let c = Cancel::default();
    let c2 = c.clone();
    let mut o = opts(&f, ExportMode::Combined);
    o.dir = f.root.join("out-combined");
    let r = export(&f.store, &f.project, &o, &c, &mut |p| {
        if p.done >= 1 {
            c2.cancel();
        }
    });
    assert!(r.cancelled && r.written.is_empty());
    assert!(all_files(&o.dir).is_empty());
}

#[test]
fn progress_counts_up_to_the_total_with_readable_messages() {
    let f = fixture("progress", &["Open", "Edit", "Save"]);
    for mode in [ExportMode::Separate, ExportMode::Combined] {
        let mut seen = Vec::new();
        let mut o = opts(&f, mode);
        o.dir = f.root.join(format!("out-{mode:?}"));
        let r = export(&f.store, &f.project, &o, &Cancel::default(), &mut |p| {
            seen.push(p)
        });
        assert!(r.is_complete());
        assert!(seen.len() >= 3);
        assert!(seen
            .iter()
            .all(|p| p.done <= p.total && !p.message.is_empty()));
        assert!(
            seen.windows(2).all(|w| w[0].done <= w[1].done),
            "progress never goes backwards: {seen:?}"
        );
        let last = seen.last().unwrap();
        assert_eq!((last.done, last.message.as_str()), (last.total, "Done"));
        assert!(
            seen.iter().any(|p| p.message.contains("Open")),
            "step titles show up in the messages"
        );
    }
}

#[test]
fn exporting_never_touches_the_project() {
    let f = fixture("untouched", &["A", "B"]);
    let snapshot = |d: &Path| -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = walk(d)
            .into_iter()
            .map(|p| {
                (
                    p.strip_prefix(d).unwrap().to_string_lossy().into_owned(),
                    fs::read(&p).unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    };
    fn walk(d: &Path) -> Vec<PathBuf> {
        fs::read_dir(d)
            .unwrap()
            .flat_map(|e| {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p)
                } else {
                    vec![p]
                }
            })
            .collect()
    }
    let before = snapshot(f.store.dir());
    for mode in [ExportMode::Separate, ExportMode::Combined] {
        let mut o = opts(&f, mode);
        o.dir = f.root.join(format!("elsewhere-{mode:?}"));
        run(&f, &o);
    }
    assert!(
        snapshot(f.store.dir()) == before,
        "the guide folder is byte-for-byte unchanged"
    );
}

/// A plain project with the same screenshots as `fixture`.
fn plain_fixture(tag: &str, titles: &[&str]) -> Fixture {
    let mut f = fixture(tag, titles);
    f.project.kind = guide_core::ProjectKind::Plain;
    f.project.name = "Some screenshots".into();
    f.store.save(&f.project).unwrap();
    f
}

#[test]
fn a_plain_project_exports_its_screenshots_as_separate_images() {
    let f = plain_fixture("plain-separate", &["Before", "After"]);
    for include_text in [false, true] {
        let mut o = opts(&f, ExportMode::Separate);
        o.include_text = include_text;
        o.replace_existing = true;
        let r = run(&f, &o);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(names(&r), ["01-before.png", "02-after.png"]);
    }
    fs::remove_dir_all(&f.root).unwrap();
}

#[test]
fn a_plain_project_cannot_be_exported_as_one_combined_image() {
    let f = plain_fixture("plain-combined", &["Before"]);
    let r = run(&f, &opts(&f, ExportMode::Combined));
    assert!(r.written.is_empty());
    assert!(
        r.problems[0].message.contains("Only a guide"),
        "{:?}",
        r.problems
    );
    assert!(!f.root.join("out").exists(), "nothing was created");
    fs::remove_dir_all(&f.root).unwrap();
}

/// How many pixels of the guide's badge colour an exported image holds.
fn badge_pixels(path: &Path) -> usize {
    image::open(path)
        .unwrap()
        .to_rgba8()
        .pixels()
        .filter(|p| p.0[..3] == [40, 110, 200])
        .count()
}

#[test]
fn text_in_separate_images_shows_the_step_number_only_for_a_guide() {
    let guide = fixture("text-numbers-guide", &["Open"]);
    let mut o = opts(&guide, ExportMode::Separate);
    o.include_text = true;
    let r = run(&guide, &o);
    assert!(
        badge_pixels(&r.written[0].path) > 0,
        "a guide draws the number badge"
    );
    let plain = plain_fixture("text-numbers-plain", &["Open"]);
    let mut o = opts(&plain, ExportMode::Separate);
    o.include_text = true;
    let r = run(&plain, &o);
    assert_eq!(
        badge_pixels(&r.written[0].path),
        0,
        "a plain project has no step numbers"
    );
    fs::remove_dir_all(&guide.root).unwrap();
    fs::remove_dir_all(&plain.root).unwrap();
}

#[test]
fn a_step_that_hides_its_description_gets_a_shorter_image_with_text() {
    let mut f = fixture("text-step-layout", &["Open", "Save"]);
    let hidden = f.project.items[0].id;
    f.project.guide_mut().unwrap().step_layouts.insert(
        hidden,
        guide_core::StepLayout {
            max_width: None,
            show_description: Some(false),
        },
    );
    let mut o = opts(&f, ExportMode::Separate);
    o.include_text = true;
    let r = run(&f, &o);
    assert!(r.is_complete(), "{:?}", r.problems);
    assert!(
        r.written[0].height < r.written[1].height,
        "the step without a description is shorter: {} vs {}",
        r.written[0].height,
        r.written[1].height
    );
    fs::remove_dir_all(&f.root).unwrap();
}

#[test]
fn a_diff_exports_as_one_image_with_the_previous_screenshot_first() {
    let mut f = fixture("diff-combined", &["Before", "After"]);
    f.project.kind = guide_core::ProjectKind::Diff(Default::default());
    f.project.name = "Settings page".into();
    f.store.save(&f.project).unwrap();
    let r = run(&f, &opts(&f, ExportMode::Combined));
    assert!(r.is_complete(), "{:?}", r.problems);
    assert_eq!(names(&r), ["settings-page.png"]);
    let layout = compute_diff_layout(&f.project, &f.store.item_sizes(&f.project)).unwrap();
    assert_eq!(
        (r.written[0].width, r.written[0].height),
        (layout.width, layout.height)
    );
    for (item, colour) in layout.images.iter().zip(COLOURS) {
        assert_eq!(
            px(
                &r.written[0].path,
                (item.rect.x + item.rect.w / 2.0) as u32,
                (item.rect.y + item.rect.h / 2.0) as u32
            ),
            colour
        );
    }
    fs::remove_dir_all(&f.root).unwrap();
}
