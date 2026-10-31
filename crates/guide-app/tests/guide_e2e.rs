//! End-to-end test of the guide window through the real binary: fake mouse and
//! keyboard on a virtual X server, then inspection of what was saved to disk.
//!
//! `scripts/test-guide.sh` provides Xvnc and Openbox. Skipped unless SNAPFORGE_E2E=1.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread::sleep,
    time::{Duration, Instant},
};

use guide_capture::{detect_backend, WindowInfo};
use guide_core::{ItemId, NewItem, Project};
use guide_hotkeys::keysym_for;
use guide_storage::ProjectStore;
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{ClientMessageEvent, ConnectionExt as _, EventMask},
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
};

const CONTROL_L: u32 = 0xFFE3;
const ALT_L: u32 = 0xFFE9;
const SHIFT_L: u32 = 0xFFE1;

struct Input {
    conn: RustConnection,
    root: u32,
    min: u8,
    per: usize,
    keysyms: Vec<u32>,
}

impl Input {
    fn new() -> Self {
        let (conn, screen) = x11rb::connect(None).unwrap();
        let setup = conn.setup();
        let (root, min, max) = (
            setup.roots[screen].root,
            setup.min_keycode,
            setup.max_keycode,
        );
        let map = conn
            .get_keyboard_mapping(min, max - min + 1)
            .unwrap()
            .reply()
            .unwrap();
        Self {
            conn,
            root,
            min,
            per: map.keysyms_per_keycode.into(),
            keysyms: map.keysyms,
        }
    }

    fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) {
        self.conn
            .xtest_fake_input(kind, detail, 0, self.root, x, y, 0)
            .unwrap();
        self.conn.flush().unwrap();
        sleep(Duration::from_millis(25));
    }

    fn key_raw(&self, keysym: u32, press: bool) {
        let kc = (self.min..=255)
            .find(|kc| self.keysyms.get(usize::from(kc - self.min) * self.per) == Some(&keysym))
            .expect("keysym in keymap");
        self.fake(if press { 2 } else { 3 }, kc, 0, 0);
    }

    fn tap(&self, name: &str) {
        // Escape is not a key the app lets users bind, so keysym_for does not know it.
        let ks = if name == "Escape" {
            0xFF1B
        } else {
            keysym_for(name).unwrap()
        };
        self.key_raw(ks, true);
        self.key_raw(ks, false);
        sleep(Duration::from_millis(120));
    }

    fn type_text(&self, text: &str) {
        for c in text.chars() {
            if c == ' ' {
                self.tap("Space");
            } else {
                self.tap(&c.to_string());
            }
        }
    }

    fn with_mods(&self, mods: &[u32], name: &str) {
        for m in mods {
            self.key_raw(*m, true);
        }
        self.tap(name);
        for m in mods.iter().rev() {
            self.key_raw(*m, false);
        }
        sleep(Duration::from_millis(150));
    }

    fn mv(&self, p: (i16, i16)) {
        self.fake(6, 0, p.0, p.1);
    }

    fn click(&self, p: (i16, i16)) {
        self.mv(p);
        sleep(Duration::from_millis(150));
        self.fake(4, 1, 0, 0);
        self.fake(5, 1, 0, 0);
        sleep(Duration::from_millis(250));
    }

    fn drag(&self, from: (i16, i16), to: (i16, i16)) {
        self.mv(from);
        sleep(Duration::from_millis(200));
        self.fake(4, 1, 0, 0);
        for step in 1..=16i16 {
            self.mv((
                from.0 + (to.0 - from.0) * step / 16,
                from.1 + (to.1 - from.1) * step / 16,
            ));
        }
        sleep(Duration::from_millis(300));
        self.fake(5, 1, 0, 0);
        sleep(Duration::from_millis(300));
    }

    fn close_window(&self, window: u32) {
        let atom = self
            .conn
            .intern_atom(false, b"_NET_CLOSE_WINDOW")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let ev = ClientMessageEvent::new(32, window, atom, [0u32, 2, 0, 0, 0]);
        self.conn
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                ev,
            )
            .unwrap();
        self.conn.flush().unwrap();
    }
}

struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

type Report = serde_json::Value;

fn wait_until<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Some(v) = f() {
            return v;
        }
        sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

fn read(path: &Path) -> Option<Report> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn order(r: &Report) -> Vec<u64> {
    r["order"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_u64()).collect())
        .unwrap_or_default()
}

fn rect_of(v: &serde_json::Value) -> (f64, f64, f64, f64) {
    let a = v
        .as_array()
        .unwrap_or_else(|| panic!("expected a rectangle, got {v}"));
    (
        a[0].as_f64().unwrap(),
        a[1].as_f64().unwrap(),
        a[2].as_f64().unwrap(),
        a[3].as_f64().unwrap(),
    )
}

fn centre(v: &serde_json::Value) -> (i16, i16) {
    let (x, y, w, h) = rect_of(v);
    ((x + w / 2.0) as i16, (y + h / 2.0) as i16)
}

fn row(r: &Report, id: u64) -> serde_json::Value {
    r["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("row {id} is not visible: {r}"))["rect"]
        .clone()
}

fn solid(w: u32, h: u32, c: [u8; 3]) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| {
            if (i % w) / 20 % 2 == 0 {
                [c[0], c[1], c[2], 255]
            } else {
                [255 - c[0], c[1], c[2], 255]
            }
        })
        .collect()
}

fn windows() -> Vec<WindowInfo> {
    detect_backend().windows().unwrap_or_default()
}

#[test]
fn reorder_duplicate_edit_exclude_remove_open_in_editor_and_close() {
    if std::env::var_os("SNAPFORGE_E2E").is_none() {
        eprintln!("skipped: SNAPFORGE_E2E not set");
        return;
    }
    let scale = std::env::var("SNAPFORGE_E2E_SCALE").unwrap_or_else(|_| "1".into());
    let tmp = std::env::temp_dir().join(format!("snapforge-guide-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let (proj_dir, cfg) = (tmp.join("proj.guideproj"), tmp.join("cfg"));
    let (guide_rep, editor_rep) = (tmp.join("guide.json"), tmp.join("editor.json"));
    fs::create_dir_all(cfg.join("snapforge")).unwrap();
    fs::write(
        cfg.join("snapforge/settings.json"),
        format!("{{\"out_dir\": {:?}}}", tmp.to_string_lossy()),
    )
    .unwrap();

    // A guide with four steps; the fourth one's screenshot is deleted to make a broken step.
    let mut guide = Project::new_guide("E2E guide");
    let project = ProjectStore::create(&proj_dir, &guide).unwrap();
    let mut assets = Vec::new();
    for (i, title) in ["Alpha", "Bravo", "Charlie", "Delta"].iter().enumerate() {
        let rel = project
            .add_asset_rgba(400, 300, &solid(400, 300, [40 + 50 * i as u8, 90, 160]))
            .unwrap();
        guide.add_item(
            usize::MAX,
            NewItem {
                title: (*title).into(),
                description: String::new(),
                caption: String::new(),
                image: rel.clone(),
            },
        );
        assets.push(rel);
    }
    project.save(&guide).unwrap();
    fs::remove_file(proj_dir.join(&assets[3])).unwrap();

    let _app = Guard(
        Command::new(env!("CARGO_BIN_EXE_snapforge"))
            .env("WINIT_X11_SCALE_FACTOR", &scale)
            .env("XDG_CONFIG_HOME", &cfg)
            .env("SNAPFORGE_TEST_OPEN_GUIDE", &proj_dir)
            .env("SNAPFORGE_TEST_AUTO_ADD", "1")
            .env("SNAPFORGE_TEST_GUIDE_REPORT", &guide_rep)
            .env("SNAPFORGE_TEST_EDITOR_REPORT", &editor_rep)
            .spawn()
            .expect("start snapforge"),
    );
    let mut app = _app;
    let wait = |what: &str, cond: &dyn Fn(&Report) -> bool| -> Report {
        wait_until(what, 20, || read(&guide_rep).filter(|r| cond(r)))
    };
    let input = Input::new();
    let snapshot = |name: &str| {
        let Some(dir) = std::env::var_os("SNAPFORGE_E2E_SHOT_DIR") else {
            return;
        };
        let shot = detect_backend()
            .capture(&guide_capture::CaptureRequest {
                include_cursor: false,
                mode: guide_capture::CaptureMode::FullScreen,
            })
            .unwrap();
        guide_storage::save_rgba(
            Path::new(&dir),
            name,
            guide_storage::ImageFormat::Png,
            shot.width,
            shot.height,
            &shot.rgba,
        )
        .unwrap();
    };

    let r = wait("the guide to open", &|r| {
        order(r) == [1, 2, 3, 4] && r["rows"].as_array().is_some_and(|a| a.len() >= 4)
    });
    sleep(Duration::from_millis(800));
    assert_eq!(
        r["issues"], 1,
        "the missing screenshot of step 4 is reported"
    );
    snapshot("guide-opened");

    // Select by clicking; move the selection with the arrow keys.
    let r = read(&guide_rep).unwrap();
    input.click(centre(&row(&r, 2)));
    wait("step 2 selected", &|r| r["selected"] == 2);
    input.tap("Down");
    wait("step 3 selected by Down", &|r| r["selected"] == 3);
    input.tap("Up");
    wait("step 2 selected by Up", &|r| r["selected"] == 2);

    // Drag step 1 down below step 3 (drag-and-drop reorder).
    let r = read(&guide_rep).unwrap();
    let (r1, r3) = (rect_of(&row(&r, 1)), rect_of(&row(&r, 3)));
    input.drag(
        centre(&row(&r, 1)),
        ((r1.0 + r1.2 / 2.0) as i16, (r3.1 + r3.3) as i16),
    );
    wait("the reordered list", &|r| order(r) == [2, 3, 1, 4]);
    // Autosave writes it without anyone pressing Save.
    wait("the autosave", &|r| r["dirty"] == false);
    let saved = ProjectStore::open(&proj_dir).unwrap().project;
    assert_eq!(
        saved.items.iter().map(|s| s.id.0).collect::<Vec<_>>(),
        [2, 3, 1, 4],
        "the new order is on disk"
    );
    for s in &saved.items {
        assert_eq!(
            s.image,
            assets[s.id.0 as usize - 1],
            "reordering never changes which image a step has"
        );
    }

    // Duplicate step 3 with Ctrl+D: a new step right after it that shares the image.
    let r = read(&guide_rep).unwrap();
    input.click(centre(&row(&r, 3)));
    wait("step 3 selected", &|r| r["selected"] == 3);
    input.with_mods(&[CONTROL_L], "D");
    wait("the duplicate", &|r| {
        order(r) == [2, 3, 5, 1, 4] && r["selected"] == 5
    });

    // Rename the copy and give it a description, exclude it from the export. These fields are
    // on the inspector's Step tab (the second one).
    let r = read(&guide_rep).unwrap();
    input.click(centre(&r["tabs"][1]));
    wait("the step tab", &|r| r["title"].is_array());
    let r = read(&guide_rep).unwrap();
    input.click(centre(&r["title"]));
    input.with_mods(&[CONTROL_L], "A");
    input.type_text("new title");
    let r = read(&guide_rep).unwrap();
    input.click(centre(&r["description"]));
    input.type_text("go on");
    let r = read(&guide_rep).unwrap();
    input.click(centre(&r["included"]));
    wait("the autosave of the edits", &|r| r["dirty"] == false);
    snapshot("guide-edited-step");
    sleep(Duration::from_millis(300));
    let saved = ProjectStore::open(&proj_dir).unwrap().project;
    let copy = saved.item(ItemId(5)).unwrap();
    assert_eq!(
        (
            copy.title.as_str(),
            copy.description.as_str(),
            copy.included
        ),
        ("new title", "go on", false)
    );
    assert_eq!(
        copy.image,
        saved.item(ItemId(3)).unwrap().image,
        "the duplicate shares the screenshot"
    );

    // Remove step 1 (its image is used by nobody else) and step 3 (its image is shared with the copy).
    let r = read(&guide_rep).unwrap();
    input.click(centre(&row(&r, 1)));
    wait("step 1 selected", &|r| r["selected"] == 1);
    input.tap("Delete");
    wait("the confirmation", &|r| r["confirm_remove"] == true);
    input.tap("Return");
    wait("step 1 removed", &|r| order(r) == [2, 3, 5, 4]);
    let r = read(&guide_rep).unwrap();
    input.click(centre(&row(&r, 3)));
    wait("step 3 selected again", &|r| r["selected"] == 3);
    input.tap("Delete");
    wait("the second confirmation", &|r| r["confirm_remove"] == true);
    input.tap("Return");
    wait("step 3 removed", &|r| order(r) == [2, 5, 4]);

    // A cancelled removal removes nothing.
    let r = read(&guide_rep).unwrap();
    input.click(centre(&row(&r, 2)));
    wait("step 2 selected", &|r| r["selected"] == 2);
    input.tap("Delete");
    wait("a third confirmation", &|r| r["confirm_remove"] == true);
    input.tap("Escape");
    wait("the dialog closed", &|r| r["confirm_remove"] == false);
    assert_eq!(order(&read(&guide_rep).unwrap()), [2, 5, 4]);

    // Alt+Down moves the selected step; Alt+Up puts it back.
    input.with_mods(&[ALT_L], "Down");
    wait("moved down", &|r| order(r) == [5, 2, 4]);
    input.with_mods(&[ALT_L], "Up");
    wait("moved up", &|r| order(r) == [2, 5, 4]);

    // The broken step cannot be edited; the app says why instead of failing.
    let r = read(&guide_rep).unwrap();
    input.click(centre(&row(&r, 4)));
    wait("the broken step selected", &|r| r["selected"] == 4);
    input.tap("Return");
    wait("the explanation", &|r| {
        r["status"]
            .as_str()
            .is_some_and(|s| s.contains("Cannot edit"))
    });

    // Open step 2 in the annotation editor, draw a rectangle, press Done.
    let r = read(&guide_rep).unwrap();
    input.click(centre(&row(&r, 2)));
    wait("step 2 selected for editing", &|r| r["selected"] == 2);
    input.tap("Return");
    let er = wait_until("the editor to open", 20, || {
        read(&editor_rep).filter(|r| r["scale"].as_f64().unwrap_or(0.0) > 0.0)
    });
    sleep(Duration::from_millis(800));
    let er = read(&editor_rep).unwrap_or(er);
    let (zoom, origin) = (
        er["scale"].as_f64().unwrap(),
        (
            er["origin"][0].as_f64().unwrap(),
            er["origin"][1].as_f64().unwrap(),
        ),
    );
    let at = |x: f64, y: f64| ((origin.0 + x * zoom) as i16, (origin.1 + y * zoom) as i16);
    input.tap("R");
    input.drag(at(50.0, 50.0), at(250.0, 180.0));
    wait_until("the rectangle", 10, || {
        read(&editor_rep).filter(|r| r["annotations"] == 1)
    });
    input.with_mods(&[CONTROL_L], "W"); // Done
    wait("back in the guide with the change", &|r| {
        r["dirty"] == true || r["status"].as_str().is_some()
    });
    wait("the autosave of the annotations", &|r| r["dirty"] == false);
    sleep(Duration::from_millis(300));
    let saved = ProjectStore::open(&proj_dir).unwrap().project;
    let annotated = saved.item(ItemId(2)).unwrap();
    assert_eq!(
        annotated.annotations.annotations.len(),
        1,
        "the editor's changes went into the guide step"
    );
    match &annotated.annotations.annotations[0].kind {
        guide_core::Kind::Rectangle { rect } => {
            assert!(
                (f64::from(rect.x) - 50.0).abs() <= 6.0 && (f64::from(rect.w) - 200.0).abs() <= 8.0,
                "{rect:?}"
            );
        }
        k => panic!("expected the rectangle, got {k:?}"),
    }

    // With "add captures to the guide" on, a capture becomes a new step right after the selected one.
    input.with_mods(&[CONTROL_L, SHIFT_L], "F");
    wait("the captured step", &|r| {
        order(r) == [2, 6, 5, 4] && r["selected"] == 6
    });
    wait("the autosave of the new step", &|r| r["dirty"] == false);
    sleep(Duration::from_millis(300));
    let saved = ProjectStore::open(&proj_dir).unwrap();
    let captured = saved.project.item(ItemId(6)).unwrap();
    let image = saved
        .store
        .load_asset(&captured.image)
        .expect("the captured screenshot was stored");
    assert!(
        image.width >= 640 && image.height >= 480,
        "a full-screen capture, got {}x{}",
        image.width,
        image.height
    );
    assert_eq!(captured.title, "New step");

    // Close the window: the guide is saved and screenshots nobody uses are cleaned up.
    let win = windows()
        .into_iter()
        .find(|w| w.title == guide_core::APP_NAME)
        .expect("app window");
    input.close_window(win.id);
    wait_until("the app to quit", 15, || {
        app.0.try_wait().unwrap().map(|_| ())
    });
    let o = ProjectStore::open(&proj_dir).unwrap();
    assert_eq!(
        o.project.items.iter().map(|s| s.id.0).collect::<Vec<_>>(),
        [2, 6, 5, 4]
    );
    assert!(
        proj_dir
            .join(&o.project.item(ItemId(6)).unwrap().image)
            .exists(),
        "the captured step keeps its screenshot"
    );
    assert!(
        !proj_dir.join(&assets[0]).exists(),
        "step 1's screenshot was used by nobody and is gone"
    );
    assert!(
        proj_dir.join(&assets[1]).exists(),
        "step 2's screenshot is in use"
    );
    assert!(
        proj_dir.join(&assets[2]).exists(),
        "step 3 was removed but its copy still uses the screenshot"
    );
    assert_eq!(
        o.issues.len(),
        1,
        "only the already-missing screenshot is a problem: {:?}",
        o.issues
    );
    let _ = (PathBuf::new(), fs::remove_dir_all(&tmp));
}
