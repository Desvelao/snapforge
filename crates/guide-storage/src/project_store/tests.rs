use std::{fs, os::unix::fs::symlink, path::PathBuf, time::Instant};

use guide_core::{Editor, Kind, NewItem, PixelRect, Point, Project, Style};
use serde_json::json;

use super::*;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("snapforge-guide-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    d
}

fn pixels(w: u32, h: u32, seed: u8) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| [(i % 251) as u8 ^ seed, (i / 3 % 256) as u8, seed, 255])
        .collect()
}

fn step(g: &mut Project, title: &str, image: &str) -> ItemId {
    g.add_item(
        usize::MAX,
        NewItem {
            title: title.into(),
            description: format!("About {title}"),
            caption: String::new(),
            image: image.into(),
        },
    )
}

/// A project with `n` steps, each with its own 8x8 screenshot.
fn project_with(tag: &str, n: u8) -> (ProjectStore, Project, PathBuf) {
    let d = dir(tag);
    let mut g = Project::new_guide("My guide");
    g.guide_mut().unwrap().introduction = "How to do the thing".into();
    let project = ProjectStore::create(&d, &g).unwrap();
    for i in 0..n {
        let rel = project.add_asset_rgba(8, 8, &pixels(8, 8, i)).unwrap();
        step(&mut g, &format!("Step {}", i + 1), &rel);
    }
    project.save(&g).unwrap();
    (project, g, d)
}

fn files(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn create_then_open_round_trips_everything() {
    let (_, g, d) = project_with("roundtrip", 3);
    assert!(d.join(ASSETS_DIR).is_dir() && d.join(EXPORTS_DIR).is_dir());
    let o = ProjectStore::open(&d).unwrap();
    assert_eq!(o.project, g);
    assert!(o.issues.is_empty(), "{:?}", o.issues);
    assert_eq!(o.notice, None);
    assert_eq!(o.project.project_id, g.project_id);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn create_refuses_to_overwrite_an_existing_project() {
    let (_, g, d) = project_with("exists", 1);
    let before = fs::read(d.join(MANIFEST_FILE)).unwrap();
    assert!(matches!(
        ProjectStore::create(&d, &Project::new_guide("other")),
        Err(ProjectError::AlreadyExists(_))
    ));
    assert!(fs::read(d.join(MANIFEST_FILE)).unwrap() == before);
    assert_eq!(ProjectStore::open(&d).unwrap().project, g);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_project_can_be_moved_and_copied() {
    let (_, g, a) = project_with("move-a", 2);
    let b = dir("move-b");
    fs::rename(&a, &b).unwrap();
    let o = ProjectStore::open(&b).unwrap();
    assert_eq!(o.project, g);
    assert!(o.issues.is_empty());
    assert_eq!(
        o.store.load_asset(&o.project.items[0].image).unwrap().width,
        8
    );
    fs::remove_dir_all(&b).unwrap();
}

#[test]
fn saving_is_atomic_leaves_no_temp_files_and_keeps_the_previous_save() {
    let (project, mut g, d) = project_with("atomic", 2);
    let first = fs::read_to_string(d.join(MANIFEST_FILE)).unwrap();
    g.name = "Renamed".into();
    project.save(&g).unwrap();
    assert_eq!(
        fs::read_to_string(d.join(BACKUP_FILE)).unwrap(),
        first,
        "the backup is the previous save"
    );
    assert!(fs::read_to_string(d.join(MANIFEST_FILE))
        .unwrap()
        .contains("Renamed"));
    let names = files(&d);
    assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn an_invalid_guide_is_refused_and_the_saved_file_is_untouched() {
    let (project, mut g, d) = project_with("refuse", 1);
    let before = fs::read(d.join(MANIFEST_FILE)).unwrap();
    g.items[0].image = "../../etc/passwd".into();
    assert!(matches!(
        project.save(&g),
        Err(ProjectError::InvalidProject(_))
    ));
    assert!(fs::read(d.join(MANIFEST_FILE)).unwrap() == before);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_damaged_manifest_is_recovered_from_the_backup_without_losing_the_damaged_file() {
    let (project, mut g, d) = project_with("recover", 2);
    g.name = "Second save".into();
    project.save(&g).unwrap();
    g.name = "Third save".into();
    project.save(&g).unwrap(); // backup now holds "Second save"
    fs::write(d.join(MANIFEST_FILE), "{ \"schema_ver").unwrap(); // truncated
    let o = ProjectStore::open(&d).unwrap();
    assert_eq!(o.project.name, "Second save", "restored the previous save");
    let notice = o.notice.expect("the user is told about the recovery");
    assert!(
        notice.contains(BACKUP_FILE) && notice.contains(DAMAGED_FILE),
        "{notice}"
    );
    assert_eq!(
        fs::read_to_string(d.join(DAMAGED_FILE)).unwrap(),
        "{ \"schema_ver",
        "the damaged file is kept"
    );
    // The restored manifest is valid, and saving again must not replace the good backup with garbage.
    assert_eq!(ProjectStore::open(&d).unwrap().project.name, "Second save");
    o.store.save(&o.project).unwrap();
    assert!(read_manifest(&d.join(BACKUP_FILE)).is_ok());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_second_recovery_keeps_the_first_damaged_file_and_leaves_no_temp_files() {
    let (project, mut g, d) = project_with("recover-twice", 1);
    g.name = "Second save".into();
    project.save(&g).unwrap();
    fs::write(d.join(MANIFEST_FILE), "first damage").unwrap();
    ProjectStore::open(&d).unwrap();
    // Save again so the backup is good, damage the manifest a second time.
    let again = ProjectStore::open(&d).unwrap();
    again.store.save(&again.project).unwrap();
    fs::write(d.join(MANIFEST_FILE), "second damage").unwrap();
    let o = ProjectStore::open(&d).unwrap();
    let second = format!("{DAMAGED_FILE}.1");
    assert!(o.notice.unwrap().contains(&second));
    assert_eq!(
        fs::read_to_string(d.join(DAMAGED_FILE)).unwrap(),
        "first damage",
        "the first damaged file is not overwritten"
    );
    assert_eq!(
        fs::read_to_string(d.join(&second)).unwrap(),
        "second damage"
    );
    assert!(
        files(&d).iter().all(|f| !f.ends_with(".tmp")),
        "{:?}",
        files(&d)
    );
    assert!(read_manifest(&d.join(MANIFEST_FILE)).is_ok());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_missing_manifest_with_a_backup_is_recovered() {
    let (project, g, d) = project_with("missing-manifest", 1);
    project.save(&g).unwrap();
    fs::remove_file(d.join(MANIFEST_FILE)).unwrap();
    let o = ProjectStore::open(&d).unwrap();
    assert_eq!(o.project.project_id, g.project_id);
    assert!(o.notice.is_some());
    assert!(d.join(MANIFEST_FILE).exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn when_both_files_are_damaged_the_error_names_the_file_and_nothing_is_deleted() {
    let (project, g, d) = project_with("both-bad", 1);
    project.save(&g).unwrap();
    fs::write(d.join(MANIFEST_FILE), "nope").unwrap();
    fs::write(d.join(BACKUP_FILE), "also nope").unwrap();
    let err = ProjectStore::open(&d).unwrap_err();
    assert!(matches!(err, ProjectError::Corrupt { .. }), "{err}");
    assert!(err.to_string().contains(MANIFEST_FILE));
    assert_eq!(fs::read_to_string(d.join(MANIFEST_FILE)).unwrap(), "nope");
    assert_eq!(
        fs::read_to_string(d.join(BACKUP_FILE)).unwrap(),
        "also nope"
    );
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn leftover_temp_files_from_an_interrupted_save_do_not_matter() {
    let (_, g, d) = project_with("leftover", 1);
    fs::write(d.join(".project.json.99999.tmp"), "half written").unwrap();
    assert_eq!(ProjectStore::open(&d).unwrap().project, g);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_guide_from_a_newer_version_is_refused_and_not_recovered_over() {
    let (project, g, d) = project_with("newer", 1);
    project.save(&g).unwrap();
    let mut v: Value =
        serde_json::from_str(&fs::read_to_string(d.join(MANIFEST_FILE)).unwrap()).unwrap();
    v["schema_version"] = json!(99);
    let newer = serde_json::to_string(&v).unwrap();
    fs::write(d.join(MANIFEST_FILE), &newer).unwrap();
    let err = ProjectStore::open(&d).unwrap_err();
    assert!(err.to_string().contains("newer version"), "{err}");
    assert_eq!(
        fs::read_to_string(d.join(MANIFEST_FILE)).unwrap(),
        newer,
        "left exactly as it was"
    );
    assert!(!d.join(DAMAGED_FILE).exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_folder_without_a_project_is_reported_as_such() {
    let d = dir("empty");
    fs::create_dir_all(&d).unwrap();
    assert!(matches!(
        ProjectStore::open(&d),
        Err(ProjectError::NotAProject(_))
    ));
    assert!(matches!(
        ProjectStore::open(&d.join("nope")),
        Err(ProjectError::NotAProject(_))
    ));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn migrations_run_in_order_and_are_reported() {
    fn v1_to_v2(v: &mut Value) -> Result<(), String> {
        v["title"] = json!(format!("{} (v2)", v["title"].as_str().unwrap_or("")));
        Ok(())
    }
    fn v2_to_v3(v: &mut Value) -> Result<(), String> {
        v["introduction"] = json!("added in v3");
        Ok(())
    }
    fn broken(_: &mut Value) -> Result<(), String> {
        Err("cannot".into())
    }
    let table = [
        Migration {
            from: 1,
            note: "renamed",
            apply: v1_to_v2,
        },
        Migration {
            from: 2,
            note: "added intro",
            apply: v2_to_v3,
        },
    ];
    let mut v = json!({"schema_version": 1, "title": "T"});
    let notes = migrate_with(&mut v, 3, &table).unwrap();
    assert_eq!(v["schema_version"], 3);
    assert_eq!(v["title"], "T (v2)");
    assert_eq!(v["introduction"], "added in v3");
    assert_eq!(notes.len(), 2);
    assert!(notes[0].contains("1 to 2") && notes[1].contains("2 to 3"));

    let mut current = json!({"schema_version": 3});
    assert_eq!(
        migrate_with(&mut current, 3, &table).unwrap(),
        Vec::<String>::new()
    );
    assert!(migrate_with(&mut json!({"schema_version": 4}), 3, &table)
        .unwrap_err()
        .contains("newer version"));
    assert!(migrate_with(&mut json!({"schema_version": 0}), 3, &table).is_err());
    assert!(migrate_with(&mut json!({"title": "no version"}), 3, &table).is_err());
    assert!(migrate_with(
        &mut json!({"schema_version": 1}),
        3,
        &[Migration {
            from: 2,
            note: "",
            apply: v2_to_v3
        }]
    )
    .unwrap_err()
    .contains("no way to upgrade"));
    assert!(migrate_with(
        &mut json!({"schema_version": 1}),
        2,
        &[Migration {
            from: 1,
            note: "",
            apply: broken
        }]
    )
    .unwrap_err()
    .contains("failed"));
}

#[test]
fn unsafe_image_paths_in_a_manifest_refuse_to_open() {
    let (project, g, d) = project_with("unsafe", 1);
    project.save(&g).unwrap();
    for bad in [
        "../outside.png",
        "/etc/passwd",
        "assets/../../etc/passwd",
        "assets/a/b.png",
        "assets\\x.png",
    ] {
        let mut v: Value =
            serde_json::from_str(&fs::read_to_string(d.join(MANIFEST_FILE)).unwrap()).unwrap();
        v["items"][0]["image"] = json!(bad);
        fs::write(d.join(MANIFEST_FILE), serde_json::to_string(&v).unwrap()).unwrap();
        fs::remove_file(d.join(BACKUP_FILE)).ok();
        let err = ProjectStore::open(&d).unwrap_err();
        assert!(
            matches!(err, ProjectError::Invalid { .. })
                || matches!(err, ProjectError::Corrupt { .. }),
            "{bad}: {err}"
        );
        assert!(
            err.to_string().contains("unsafe image path"),
            "{bad}: {err}"
        );
    }
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn asset_path_resolution_rejects_traversal() {
    let (project, _, d) = project_with("resolve", 1);
    assert!(
        project.asset_path("assets/anything.png").is_ok(),
        "a missing file is not unsafe"
    );
    for bad in ["../x", "/etc/passwd", "assets/../x", "assets/", "x.png"] {
        assert!(
            matches!(
                project.asset_path(bad),
                Err(ProjectError::UnsafePath { .. })
            ),
            "{bad}"
        );
    }
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_symlink_that_leaves_the_project_is_never_followed() {
    let (project, mut g, d) = project_with("symlink", 1);
    let outside = dir("symlink-outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.png"), "top secret").unwrap();
    symlink(
        outside.join("secret.png"),
        d.join(ASSETS_DIR).join("shot-evil.png"),
    )
    .unwrap();
    let id = step(&mut g, "Evil", "assets/shot-evil.png");
    project.save(&g).unwrap();
    let o = ProjectStore::open(&d).unwrap();
    let issue = o
        .issues
        .iter()
        .find(|i| i.item == id)
        .expect("the escaping asset is reported");
    assert_eq!(issue.kind, IssueKind::AssetEscapes);
    assert!(matches!(
        o.store.load_asset("assets/shot-evil.png"),
        Err(ProjectError::UnsafePath { .. })
    ));
    // Pruning must not follow or delete through the link either.
    o.store
        .prune_unreferenced_assets(&Project::new_guide("empty"))
        .unwrap();
    assert_eq!(
        fs::read_to_string(outside.join("secret.png")).unwrap(),
        "top secret"
    );
    fs::remove_dir_all(&d).unwrap();
    fs::remove_dir_all(&outside).unwrap();
}

#[test]
fn missing_and_unreadable_images_open_with_clear_issues() {
    let (project, mut g, d) = project_with("issues", 3);
    // Step 1: delete the file. Step 2: replace it with garbage. Step 3 stays fine.
    fs::remove_file(d.join(&g.items[0].image)).unwrap();
    fs::write(d.join(&g.items[1].image), "not an image").unwrap();
    g.items[0].title = "Open settings".into();
    project.save(&g).unwrap();
    let o = ProjectStore::open(&d).unwrap();
    assert_eq!(o.project.items.len(), 3, "the guide still opens");
    assert_eq!(o.issues.len(), 2, "{:?}", o.issues);
    let (missing, invalid) = (&o.issues[0], &o.issues[1]);
    assert_eq!(
        (missing.item, missing.kind),
        (g.items[0].id, IssueKind::MissingAsset)
    );
    assert!(
        missing.message.contains("Open settings") && missing.message.contains("missing"),
        "{}",
        missing.message
    );
    assert_eq!(
        (invalid.item, invalid.kind),
        (g.items[1].id, IssueKind::InvalidAsset)
    );
    assert!(
        invalid.message.contains("not a readable image"),
        "{}",
        invalid.message
    );
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn annotations_that_do_not_fit_their_image_are_reported() {
    let (project, mut g, d) = project_with("badannot", 1);
    let mut e = Editor::new(guide_core::Document::default());
    e.add(
        Kind::Line {
            from: Point::new(0.0, 0.0),
            to: Point::new(1.0, 1.0),
        },
        Style::default(),
    );
    e.set_crop(Some(PixelRect {
        x: 0,
        y: 0,
        w: 500,
        h: 500,
    })); // the image is 8x8
    g.items[0].annotations = e.document().clone();
    project.save(&g).unwrap();
    let o = ProjectStore::open(&d).unwrap();
    assert_eq!(o.issues.len(), 1);
    assert_eq!(o.issues[0].kind, IssueKind::InvalidAnnotations);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn new_assets_never_overwrite_each_other_and_round_trip_exactly() {
    let (project, _, d) = project_with("assets", 0);
    let a = project.add_asset_rgba(8, 8, &pixels(8, 8, 1)).unwrap();
    let b = project.add_asset_rgba(8, 8, &pixels(8, 8, 2)).unwrap();
    assert_ne!(a, b);
    assert!(
        asset_path_problem(&a).is_none() && asset_path_problem(&b).is_none(),
        "{a} {b}"
    );
    assert!(project.load_asset(&a).unwrap().rgba == pixels(8, 8, 1));
    assert!(project.load_asset(&b).unwrap().rgba == pixels(8, 8, 2));
    assert!(project.add_asset_rgba(0, 4, &[]).is_err());
    fs::remove_dir_all(&d).unwrap();
}

/// Minimal PNG whose header claims a huge size, so the size limit can be tested without a huge file.
fn png_claiming(width: u32, height: u32) -> Vec<u8> {
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
    let mut ihdr = Vec::new();
    ihdr.extend(b"IHDR");
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(13u32.to_be_bytes());
    png.extend(&ihdr);
    png.extend(crc32(&ihdr).to_be_bytes());
    // The decoder reads headers up to the first data chunk, so one (empty) is needed.
    png.extend(0u32.to_be_bytes());
    png.extend(b"IDAT");
    png.extend(crc32(b"IDAT").to_be_bytes());
    png
}

#[test]
fn importing_converts_to_png_leaves_the_source_alone_and_enforces_limits() {
    let (project, _, d) = project_with("import", 0);
    let src = dir("import-src");
    let jpg = save_rgba(&src, "photo", ImageFormat::Jpeg, 16, 12, &pixels(16, 12, 3)).unwrap();
    let webp = save_rgba(&src, "pic", ImageFormat::WebP, 10, 10, &pixels(10, 10, 4)).unwrap();
    let before = fs::read(&jpg).unwrap();
    let (rel, w, h) = project.import_image(&jpg).unwrap();
    assert_eq!((w, h), (16, 12));
    assert!(
        rel.starts_with("assets/shot-") && rel.ends_with(".png"),
        "{rel}"
    );
    assert_eq!(project.load_asset(&rel).unwrap().width, 16);
    assert!(
        fs::read(&jpg).unwrap() == before,
        "the imported file is not modified"
    );
    let (rel2, w2, _) = project.import_image(&webp).unwrap();
    assert_eq!(w2, 10);
    assert_ne!(rel, rel2);

    fs::write(src.join("notes.png"), "just text").unwrap();
    assert!(matches!(
        project.import_image(&src.join("notes.png")),
        Err(ProjectError::BadImage { .. })
    ));
    assert!(matches!(
        project.import_image(&src.join("missing.png")),
        Err(ProjectError::BadImage { .. })
    ));
    fs::write(src.join("huge.png"), png_claiming(20_000, 20_000)).unwrap();
    assert!(matches!(
        project.import_image(&src.join("huge.png")),
        Err(ProjectError::TooLarge { .. })
    ));
    fs::remove_dir_all(&d).unwrap();
    fs::remove_dir_all(&src).unwrap();
}

#[test]
fn deleting_a_step_keeps_an_image_another_step_still_uses() {
    let (project, mut g, d) = project_with("prune", 2);
    let shared = g.items[0].image.clone();
    let other = g.items[1].image.clone();
    let copy = g.duplicate_item(g.items[0].id, str::to_string).unwrap();
    project.save(&g).unwrap();
    fs::write(d.join(ASSETS_DIR).join("notes.txt"), "mine").unwrap();
    fs::write(d.join(ASSETS_DIR).join("user-photo.png"), "mine too").unwrap();

    // Delete the original of the shared image: the copy still needs it.
    let first = g.items[0].id;
    g.remove_item(first);
    assert!(project.prune_unreferenced_assets(&g).unwrap().is_empty());
    assert!(d.join(&shared).exists(), "still used by the duplicate");

    // Delete the copy too: now nothing refers to it, and only then is the file removed.
    g.remove_item(copy);
    assert_eq!(
        project.prune_unreferenced_assets(&g).unwrap(),
        vec![shared.clone()]
    );
    assert!(!d.join(&shared).exists());
    assert!(d.join(&other).exists(), "an image still in use is kept");
    assert!(
        d.join(ASSETS_DIR).join("notes.txt").exists(),
        "files this app did not create are never deleted"
    );
    assert!(d.join(ASSETS_DIR).join("user-photo.png").exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn ids_and_image_references_survive_reordering_and_a_reload() {
    let (project, mut g, d) = project_with("reorder", 4);
    let pairs = |g: &Project| -> Vec<(u64, String)> {
        g.items.iter().map(|s| (s.id.0, s.image.clone())).collect()
    };
    let before = pairs(&g);
    g.move_item(g.items[0].id, 3);
    g.move_item(g.items[2].id, 0);
    project.save(&g).unwrap();
    let o = ProjectStore::open(&d).unwrap();
    assert_eq!(o.project.project_id, g.project_id);
    let mut after = pairs(&o.project);
    assert_eq!(after, pairs(&g), "order survives a reload");
    after.sort();
    let mut expected = before;
    expected.sort();
    assert_eq!(after, expected, "every id still points at its own image");
    assert!(o.issues.is_empty());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn hundreds_of_steps_save_and_open_quickly() {
    let (project, mut g, d) = project_with("many", 3);
    let images: Vec<String> = g.items.iter().map(|s| s.image.clone()).collect();
    for i in 0..500 {
        step(&mut g, &format!("Step {i}"), &images[i % 3]);
    }
    let t = Instant::now();
    project.save(&g).unwrap();
    let o = ProjectStore::open(&d).unwrap();
    assert_eq!(o.project.items.len(), 503);
    assert!(o.issues.is_empty(), "{:?}", o.issues.len());
    assert!(t.elapsed().as_secs() < 10, "took {:?}", t.elapsed());
    fs::remove_dir_all(&d).unwrap();
}

/// Keeps docs/project-format.md honest: these are the keys it documents.
#[test]
fn the_saved_manifest_has_the_documented_structure() {
    let (_, _, d) = project_with("structure", 1);
    let v: Value =
        serde_json::from_str(&fs::read_to_string(d.join(MANIFEST_FILE)).unwrap()).unwrap();
    let keys = |v: &Value| -> Vec<String> {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(
        keys(&v),
        [
            "items",
            "kind",
            "name",
            "next_item_id",
            "project_id",
            "schema_version"
        ]
    );
    assert_eq!(
        keys(&v["kind"]),
        [
            "composition",
            "final_notes",
            "introduction",
            "step_layouts",
            "type"
        ]
    );
    assert_eq!(v["kind"]["type"], "guide");
    assert_eq!(v["schema_version"], 1);
    assert!(v["project_id"].as_str().unwrap().starts_with("p-"));
    assert_eq!(
        keys(&v["items"][0]),
        [
            "annotations",
            "caption",
            "description",
            "id",
            "image",
            "included",
            "title"
        ]
    );
    assert_eq!(v["items"][0]["included"], true);
    assert!(v["items"][0]["image"]
        .as_str()
        .unwrap()
        .starts_with("assets/shot-"));
    assert_eq!(
        keys(&v["items"][0]["annotations"]),
        ["annotations", "crop", "next_id", "version"]
    );
    assert_eq!(
        keys(&v["kind"]["composition"]),
        [
            "background",
            "description_placement",
            "font_size",
            "image_align",
            "image_scaling",
            "layout",
            "margin",
            "max_width",
            "show_descriptions",
            "show_final_notes",
            "show_guide_title",
            "show_introduction",
            "show_numbers",
            "show_titles",
            "spacing",
            "text_color"
        ]
    );
    assert_eq!(v["kind"]["composition"]["layout"], "Vertical");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn item_sizes_use_the_crop_and_skip_unreadable_images() {
    let (project, mut g, d) = project_with("sizes", 3);
    // Step 2 is cropped, step 3's file is damaged.
    let mut e = Editor::new(guide_core::Document::default());
    e.set_crop(Some(PixelRect {
        x: 1,
        y: 1,
        w: 5,
        h: 4,
    }));
    g.items[1].annotations = e.document().clone();
    fs::write(d.join(&g.items[2].image), "garbage").unwrap();
    let sizes = project.item_sizes(&g);
    assert_eq!(sizes.get(&g.items[0].id), Some(&(8, 8)));
    assert_eq!(
        sizes.get(&g.items[1].id),
        Some(&(5, 4)),
        "the crop decides the size"
    );
    assert_eq!(
        sizes.get(&g.items[2].id),
        None,
        "unreadable images are absent, not an error"
    );
    fs::remove_dir_all(&d).unwrap();
}

// ---- Phase 7: error messages for every way a project can be wrong ---------------------------------

/// Opens a project whose manifest was replaced by `manifest`, removing the backup so recovery
/// cannot hide the real error.
fn open_with_manifest(tag: &str, manifest: &[u8]) -> Result<Opened, ProjectError> {
    let (_, _, d) = project_with(tag, 1);
    fs::remove_file(d.join(BACKUP_FILE)).ok();
    fs::write(d.join(MANIFEST_FILE), manifest).unwrap();
    let r = ProjectStore::open(&d);
    fs::remove_dir_all(&d).ok();
    r
}

fn message(r: Result<Opened, ProjectError>) -> String {
    r.expect_err("expected an error").to_string()
}

#[test]
fn a_manifest_that_is_not_json_says_so_with_the_file_and_the_position() {
    let m = message(open_with_manifest(
        "e-syntax",
        b"{ \"schema_version\": 1,\n  \"name\": oops }",
    ));
    assert!(m.contains(MANIFEST_FILE) && m.contains("damaged"), "{m}");
    assert!(
        m.contains("line 2"),
        "the position of the mistake is given: {m}"
    );
    let m = message(open_with_manifest("e-empty", b""));
    assert!(m.contains(MANIFEST_FILE) && m.contains("damaged"), "{m}");
}

#[test]
fn a_manifest_with_the_wrong_shape_names_the_problem() {
    let m = message(open_with_manifest(
        "e-name",
        br#"{"schema_version":1,"project_id":"p-1","kind":{"type":"plain"},"items":[]}"#,
    ));
    assert!(m.contains("missing field `name`"), "{m}");
    let m = message(open_with_manifest(
        "e-steps",
        br#"{"schema_version":1,"project_id":"p-1","name":"T","kind":{"type":"plain"},"items":"nope"}"#,
    ));
    assert!(m.contains("items") || m.contains("sequence"), "{m}");
    let m = message(open_with_manifest(
        "e-version-type",
        br#"{"schema_version":"1","project_id":"p-1","name":"T","kind":{"type":"plain"},"items":[]}"#,
    ));
    assert!(
        m.contains("schema_version") && m.contains("whole number"),
        "{m}"
    );
    let m = message(open_with_manifest(
        "e-version-missing",
        br#"{"project_id":"p-1","name":"T","kind":{"type":"plain"},"items":[]}"#,
    ));
    assert!(m.contains("schema_version"), "{m}");
    let m = message(open_with_manifest(
        "e-version-zero",
        br#"{"schema_version":0,"project_id":"p-1","name":"T","kind":{"type":"plain"},"items":[]}"#,
    ));
    assert!(
        m.contains("schema_version") && m.contains("not valid"),
        "{m}"
    );
    let m = message(open_with_manifest(
        "e-old-layout",
        br#"{"schema_version":1,"project_id":"p-1","title":"T","steps":[]}"#,
    ));
    assert!(m.contains("older layout"), "{m}");
    let m = message(open_with_manifest("e-array", b"[1,2,3]"));
    assert!(m.contains(MANIFEST_FILE), "{m}");
}

#[test]
fn a_newer_format_is_named_as_such() {
    let m = message(open_with_manifest(
        "e-newer",
        br#"{"schema_version":42,"project_id":"p-1","title":"T","steps":[]}"#,
    ));
    assert!(m.contains("newer version") && m.contains("42"), "{m}");
}

#[test]
fn a_manifest_that_cannot_be_read_is_not_described_as_a_write_failure() {
    // A folder where project.json should be.
    let (_, _, d) = project_with("e-dir", 1);
    fs::remove_file(d.join(BACKUP_FILE)).ok();
    fs::remove_file(d.join(MANIFEST_FILE)).unwrap();
    fs::create_dir_all(d.join(MANIFEST_FILE)).unwrap();
    let m = message(ProjectStore::open(&d));
    assert!(
        m.contains("cannot read") && m.contains(MANIFEST_FILE),
        "{m}"
    );
    assert!(
        !m.contains("cannot write"),
        "reading must not be reported as writing: {m}"
    );
    fs::remove_dir_all(&d).unwrap();

    // A file the user may not read (skipped when the tests run with privileges that ignore permissions).
    use std::os::unix::fs::PermissionsExt;
    let (_, _, d) = project_with("e-perm", 1);
    fs::remove_file(d.join(BACKUP_FILE)).ok();
    fs::set_permissions(d.join(MANIFEST_FILE), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(d.join(MANIFEST_FILE)).is_err() {
        let m = message(ProjectStore::open(&d));
        assert!(m.contains("cannot read") && m.contains("ermission"), "{m}");
    }
    fs::set_permissions(d.join(MANIFEST_FILE), fs::Permissions::from_mode(0o644)).unwrap();
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn an_editor_added_byte_order_mark_does_not_break_the_file() {
    let (_, g, d) = project_with("bom", 1);
    let json = fs::read(d.join(MANIFEST_FILE)).unwrap();
    let mut with_bom = vec![0xEF, 0xBB, 0xBF];
    with_bom.extend(json);
    fs::write(d.join(MANIFEST_FILE), with_bom).unwrap();
    assert_eq!(ProjectStore::open(&d).unwrap().project, g);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn an_absurdly_large_manifest_is_refused_instead_of_read_into_memory() {
    let (_, _, d) = project_with("e-big", 1);
    fs::remove_file(d.join(BACKUP_FILE)).ok();
    let f = fs::OpenOptions::new()
        .write(true)
        .open(d.join(MANIFEST_FILE))
        .unwrap();
    f.set_len(200 * 1024 * 1024).unwrap(); // sparse: costs no disk space
    drop(f);
    let m = message(ProjectStore::open(&d));
    assert!(m.contains("too large") && m.contains(MANIFEST_FILE), "{m}");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn an_annotation_type_from_a_newer_version_is_explained() {
    let (project, mut g, d) = project_with("e-kind", 1);
    project.save(&g).unwrap();
    let mut v: Value =
        serde_json::from_str(&fs::read_to_string(d.join(MANIFEST_FILE)).unwrap()).unwrap();
    v["items"][0]["annotations"]["annotations"] = json!([{"id": 1, "kind": {"type": "Spotlight", "radius": 5}, "style": {"color": [0,0,0,255], "width": 1.0, "fill": null}}]);
    g.name = "x".into();
    fs::remove_file(d.join(BACKUP_FILE)).ok();
    fs::write(d.join(MANIFEST_FILE), serde_json::to_string(&v).unwrap()).unwrap();
    let m = message(ProjectStore::open(&d));
    assert!(m.contains("Spotlight") && m.contains("newer"), "{m}");
    fs::remove_dir_all(&d).unwrap();
}

// ---- Phase 7: forward compatibility, golden file, portability -----------------------------------------

#[test]
fn fields_this_version_does_not_know_survive_open_and_save() {
    let (project, g, d) = project_with("extras", 1);
    project.save(&g).unwrap();
    let mut v: Value =
        serde_json::from_str(&fs::read_to_string(d.join(MANIFEST_FILE)).unwrap()).unwrap();
    v["future_project_field"] = json!({"nested": [1, 2, 3]});
    v["items"][0]["future_item_field"] = json!("keep me");
    v["items"][0]["annotations"]["future_doc_field"] = json!(7);
    v["kind"]["composition"]["future_setting"] = json!(true);
    fs::write(
        d.join(MANIFEST_FILE),
        serde_json::to_string_pretty(&v).unwrap(),
    )
    .unwrap();

    let mut o = ProjectStore::open(&d).unwrap();
    o.project.name = "Edited by this version".into(); // an ordinary edit, then save
    o.store.save(&o.project).unwrap();

    let saved: Value =
        serde_json::from_str(&fs::read_to_string(d.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert_eq!(saved["name"], "Edited by this version");
    assert_eq!(
        saved["future_project_field"],
        json!({"nested": [1, 2, 3]}),
        "nothing a newer build wrote was dropped"
    );
    assert_eq!(saved["items"][0]["future_item_field"], "keep me");
    assert_eq!(saved["items"][0]["annotations"]["future_doc_field"], 7);
    assert_eq!(saved["kind"]["composition"]["future_setting"], true);
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn out_of_range_settings_are_corrected_on_open_and_the_user_is_told() {
    let (project, g, d) = project_with("range", 1);
    project.save(&g).unwrap();
    let mut v: Value =
        serde_json::from_str(&fs::read_to_string(d.join(MANIFEST_FILE)).unwrap()).unwrap();
    v["kind"]["composition"]["margin"] = json!(99999);
    v["kind"]["composition"]["layout"] = json!({"Grid": {"columns": 0}});
    fs::write(d.join(MANIFEST_FILE), serde_json::to_string(&v).unwrap()).unwrap();
    let o = ProjectStore::open(&d).unwrap();
    assert_eq!(o.project.guide().unwrap().composition.margin, 2000);
    assert_eq!(
        o.project.guide().unwrap().composition.layout,
        guide_core::Layout::Grid { columns: 1 }
    );
    let notice = o.notice.expect("the user is told what was changed");
    assert!(
        notice.contains("margin") && notice.contains("99999") && notice.contains("columns"),
        "{notice}"
    );
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_copied_project_and_a_project_opened_through_a_symlink_work_like_the_original() {
    let (_, g, a) = project_with("copy-src", 3);
    let b = dir("copy-dst");
    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for e in fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let target = to.join(e.file_name());
            if e.path().is_dir() {
                copy_tree(&e.path(), &target);
            } else {
                fs::copy(e.path(), &target).unwrap();
            }
        }
    }
    copy_tree(&a, &b);
    fs::remove_dir_all(&a).unwrap(); // prove the copy does not lean on the original
    let o = ProjectStore::open(&b).unwrap();
    assert_eq!(o.project, g);
    assert!(o.issues.is_empty(), "{:?}", o.issues);
    for s in &o.project.items {
        assert!(o.store.load_asset(&s.image).is_ok());
    }
    // Opened through a link to the folder (a shortcut, a mounted share): the safety check must not reject it.
    let link = dir("copy-link");
    symlink(&b, &link).unwrap();
    let o = ProjectStore::open(&link).unwrap();
    assert!(o.issues.is_empty(), "{:?}", o.issues);
    assert!(o.store.load_asset(&o.project.items[0].image).is_ok());
    fs::remove_file(&link).unwrap();
    fs::remove_dir_all(&b).unwrap();
}

#[test]
fn the_manifest_contains_no_embedded_images() {
    let (_, _, d) = project_with("noblob", 3);
    let text = fs::read_to_string(d.join(MANIFEST_FILE)).unwrap();
    assert!(text.len() < 20_000, "{} bytes for 3 steps", text.len());
    assert!(
        !text.contains("base64") && !text.contains("data:image") && !text.contains("iVBOR"),
        "images must stay in assets/"
    );
    let img = fs::read_dir(d.join(ASSETS_DIR)).unwrap().count();
    assert_eq!(img, 3, "each screenshot is a file of its own");
    assert_eq!(
        fs::read_dir(d.join(EXPORTS_DIR)).unwrap().count(),
        0,
        "exports live apart from the originals"
    );
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn item_sizes_read_only_the_image_header_so_a_combined_export_does_not_decode_twice() {
    let (project, g, d) = project_with("header-only", 1);
    // A PNG cut off right after the start of its pixel data cannot be decoded, but its size is known.
    let path = d.join(&g.items[0].image);
    let bytes = fs::read(&path).unwrap();
    let idat = bytes.windows(4).position(|w| w == b"IDAT").unwrap();
    fs::write(&path, &bytes[..idat + 4]).unwrap();
    assert!(project.load_asset(&g.items[0].image).is_err());
    assert_eq!(project.item_sizes(&g).get(&g.items[0].id), Some(&(8, 8)));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_link_in_place_of_the_assets_folder_gets_no_writes_and_no_deletions() {
    let (project, g, d) = project_with("assets-link", 1);
    let outside = dir("assets-link-outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("shot-precious.png"), "not ours to delete").unwrap();
    fs::remove_dir_all(d.join(ASSETS_DIR)).unwrap();
    symlink(&outside, d.join(ASSETS_DIR)).unwrap();

    let err = project.add_asset_rgba(8, 8, &pixels(8, 8, 1)).unwrap_err();
    assert!(matches!(err, ProjectError::UnsafePath { .. }), "{err}");
    let err = project.prune_unreferenced_assets(&g).unwrap_err();
    assert!(matches!(err, ProjectError::UnsafePath { .. }), "{err}");
    // Not even a file that does not exist yet may be resolved through the link.
    let err = project.asset_path("assets/shot-new.png").unwrap_err();
    assert!(matches!(err, ProjectError::UnsafePath { .. }), "{err}");

    assert_eq!(
        files(&outside),
        ["shot-precious.png"],
        "nothing was written or removed there"
    );
    fs::remove_dir_all(&d).unwrap();
    fs::remove_dir_all(&outside).unwrap();
}

#[test]
fn creating_a_project_over_a_linked_assets_folder_is_refused() {
    let d = dir("create-link");
    let outside = dir("create-link-outside");
    fs::create_dir_all(&d).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, d.join(EXPORTS_DIR)).unwrap();
    let err = ProjectStore::create(&d, &Project::new_guide("G")).unwrap_err();
    assert!(matches!(err, ProjectError::UnsafePath { .. }), "{err}");
    fs::remove_dir_all(&d).unwrap();
    fs::remove_dir_all(&outside).unwrap();
}

#[test]
fn asset_path_returns_the_checked_real_path() {
    let (project, g, d) = project_with("asset-real", 1);
    let real = fs::canonicalize(d.join(&g.items[0].image)).unwrap();
    assert_eq!(project.asset_path(&g.items[0].image).unwrap(), real);
    // A file that is not there yet still resolves inside the real assets folder.
    let missing = project.asset_path("assets/shot-new.png").unwrap();
    assert_eq!(
        missing,
        fs::canonicalize(d.join(ASSETS_DIR))
            .unwrap()
            .join("shot-new.png")
    );
    fs::remove_dir_all(&d).unwrap();
}
