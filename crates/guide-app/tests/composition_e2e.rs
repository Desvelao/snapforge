//! End-to-end test of the composition editor (the three-panel guide window): fake
//! mouse and keyboard on a virtual X server, comparing what the window reports
//! with what the layout engine computes on its own, and with what is saved.
//!
//! `scripts/test-guide.sh` provides Xvnc and Openbox. Skipped unless SNAPFORGE_E2E=1.
use std::{
    fs,
    path::Path,
    process::{Child, Command},
    thread::sleep,
    time::{Duration, Instant},
};

use guide_capture::{detect_backend, WindowInfo};
use guide_core::{NewItem, Project};
use guide_hotkeys::keysym_for;
use guide_render::compose::compute_layout;
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
        let ks = keysym_for(name).unwrap();
        self.key_raw(ks, true);
        self.key_raw(ks, false);
        sleep(Duration::from_millis(120));
    }

    fn select_all(&self) {
        self.key_raw(CONTROL_L, true);
        self.tap("A");
        self.key_raw(CONTROL_L, false);
        sleep(Duration::from_millis(150));
    }

    fn click(&self, p: (i16, i16)) {
        self.fake(6, 0, p.0, p.1);
        sleep(Duration::from_millis(150));
        self.fake(4, 1, 0, 0);
        self.fake(5, 1, 0, 0);
        sleep(Duration::from_millis(300));
    }

    /// Scrolls the wheel down over `at`, which moves a scroll area under the pointer.
    fn wheel_down(&self, at: (i16, i16), clicks: u32) {
        self.fake(6, 0, at.0, at.1);
        sleep(Duration::from_millis(150));
        for _ in 0..clicks {
            self.fake(4, 5, 0, 0);
            self.fake(5, 5, 0, 0);
            sleep(Duration::from_millis(80));
        }
        sleep(Duration::from_millis(500));
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

/// Scrolls the properties panel until the control reported under `key` lies well inside the app
/// window, and returns that report. A control on the window's edge or border cannot be clicked
/// reliably; where the panel's content ends depends on the interface scale.
fn scroll_into_view(input: &Input, rep: &Path, key: &str) -> Report {
    let window = || {
        guide_capture::detect_backend()
            .windows()
            .unwrap_or_default()
            .into_iter()
            .find(|w| w.title == guide_core::APP_NAME)
            .expect("app window")
            .rect
    };
    for _ in 0..12 {
        let r = wait_until(&format!("{key} in the report"), 25, || {
            read(rep).filter(|r| r[key].is_array())
        });
        let (win, c) = (window(), centre(&r[key]));
        let a = r[key].as_array().unwrap();
        let (top, bottom) = (
            a[1].as_f64().unwrap(),
            (a[1].as_f64().unwrap() + a[3].as_f64().unwrap()),
        );
        let margin = 30.0;
        if top > f64::from(win.y) + margin && bottom < f64::from(win.y) + f64::from(win.h) - margin
        {
            // The panel scrolls smoothly: only a rectangle that holds still can be clicked.
            sleep(Duration::from_millis(400));
            match read(rep) {
                Some(again) if again[key] == r[key] => return again,
                _ => continue,
            }
        }
        // The panel is the right-hand column: wheel over the control's column, mid-window.
        input.wheel_down((c.0, (win.y + win.h as i32 / 2) as i16), 2);
    }
    panic!("could not scroll {key} into view");
}

fn centre(v: &serde_json::Value) -> (i16, i16) {
    let a = v
        .as_array()
        .unwrap_or_else(|| panic!("expected a rectangle, got {v}"));
    let f = |i: usize| a[i].as_f64().unwrap();
    ((f(0) + f(2) / 2.0) as i16, (f(1) + f(3) / 2.0) as i16)
}

fn size_of(r: &Report) -> Option<(u64, u64)> {
    let a = r["preview_size"].as_array()?;
    Some((a[0].as_u64()?, a[1].as_u64()?))
}

fn solid(w: u32, h: u32, c: [u8; 3]) -> Vec<u8> {
    [c[0], c[1], c[2], 255].repeat(w as usize * h as usize)
}

fn expected(project: &ProjectStore, guide: &Project) -> (u64, u64) {
    let l = compute_layout(guide, &project.item_sizes(guide)).unwrap();
    (u64::from(l.width), u64::from(l.height))
}

#[test]
fn the_preview_matches_the_engine_and_the_controls_change_the_guide() {
    if std::env::var_os("SNAPFORGE_E2E").is_none() {
        eprintln!("skipped: SNAPFORGE_E2E not set");
        return;
    }
    let scale = std::env::var("SNAPFORGE_E2E_SCALE").unwrap_or_else(|_| "1".into());
    let tmp = std::env::temp_dir().join(format!("snapforge-comp-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let (dir, cfg, rep) = (
        tmp.join(format!("g.{}", guide_core::PROJECT_EXTENSION)),
        tmp.join("cfg"),
        tmp.join("guide.json"),
    );
    fs::create_dir_all(cfg.join("snapforge")).unwrap();
    fs::write(
        cfg.join("snapforge/settings.json"),
        format!("{{\"out_dir\": {:?}}}", tmp.to_string_lossy()),
    )
    .unwrap();

    let mut guide = Project::new_guide("Composition test");
    guide.guide_mut().unwrap().introduction = "A short introduction to this guide.".into();
    let project = ProjectStore::create(&dir, &guide).unwrap();
    for (i, (title, w, h)) in [
        ("First", 800u32, 500u32),
        ("Second", 1200, 700),
        ("Third", 600, 400),
    ]
    .into_iter()
    .enumerate()
    {
        let rel = project
            .add_asset_rgba(w, h, &solid(w, h, [60 * i as u8 + 30, 120, 200]))
            .unwrap();
        guide.add_item(
            usize::MAX,
            NewItem {
                title: title.into(),
                description: format!(
                    "Do the {} thing in the toolbar, then continue.",
                    title.to_lowercase()
                ),
                caption: String::new(),
                image: rel,
            },
        );
    }
    project.save(&guide).unwrap();

    let mut app = Guard(
        Command::new(env!("CARGO_BIN_EXE_snapforge"))
            .env("WINIT_X11_SCALE_FACTOR", &scale)
            .env("XDG_CONFIG_HOME", &cfg)
            .env("SNAPFORGE_TEST_OPEN_GUIDE", &dir)
            .env("SNAPFORGE_TEST_GUIDE_REPORT", &rep)
            .spawn()
            .expect("start snapforge"),
    );
    let wait = |what: &str, cond: &dyn Fn(&Report) -> bool| -> Report {
        let end = Instant::now() + Duration::from_secs(25);
        loop {
            if let Some(r) = read(&rep).filter(|r| cond(r)) {
                return r;
            }
            assert!(
                Instant::now() < end,
                "timed out waiting for {what}; windows: {:?}; last report: {}",
                guide_capture::detect_backend()
                    .windows()
                    .unwrap_or_default()
                    .iter()
                    .map(|w| (w.title.clone(), w.rect))
                    .collect::<Vec<_>>(),
                read(&rep).map(|r| r.to_string()).unwrap_or_default()
            );
            sleep(Duration::from_millis(100));
        }
    };
    let input = Input::new();

    // The window shows the size the engine computes for the same guide.
    let want = expected(&project, &guide);
    let r = wait("the first preview", &|r| {
        r["preview_state"] == "ready" && size_of(r).is_some()
    });
    assert_eq!(
        size_of(&r),
        Some(want),
        "the output size shown is the engine's size for this guide"
    );
    assert!(
        r["preview_scale"].as_f64().unwrap() < 1.0,
        "the quick preview is drawn smaller than the final image"
    );
    assert_eq!(
        r["boxes"].as_array().unwrap().len(),
        3,
        "every included step has a box in the preview"
    );

    // Clicking a step in the preview selects it.
    let second = r["boxes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == 2)
        .unwrap()["rect"]
        .clone();
    input.click(centre(&second));
    wait("step 2 selected from the preview", &|r| r["selected"] == 2);

    // Folding the properties away gives the preview more room; opening them again restores it.
    let before = read(&rep).unwrap();
    assert_eq!(before["right_open"], true);
    input.click(centre(&before["toggle_right"]));
    let after = wait("the properties folded", &|r| r["right_open"] == false);
    wait("the preview refitted to the wider area", &|r| {
        r["zoom"].as_f64().unwrap_or(0.0) > before["zoom"].as_f64().unwrap() * 1.1
    });
    input.click(centre(&after["toggle_right"]));
    wait("the properties back", &|r| r["right_open"] == true);

    // Change the content width through the real control: type 800 into it. It is on the
    // inspector's Layout tab (the third one).
    let r = read(&rep).unwrap();
    input.click(centre(&r["tabs"][2]));
    let r = scroll_into_view(&input, &rep, "max_width");
    input.click(centre(&r["max_width"]));
    input.select_all();
    for c in "800".chars() {
        input.tap(&c.to_string());
    }
    input.tap("Return");
    guide.guide_mut().unwrap().composition.max_width = 800;
    let want = expected(&project, &guide);
    let r = wait("the narrower layout", &|r| {
        r["preview_state"] == "ready" && size_of(r) == Some(want)
    });
    assert_eq!(
        size_of(&r).unwrap().0,
        800 + 2 * 32,
        "content width plus the two margins"
    );

    // Hide the step descriptions: the guide gets shorter, by exactly what the engine says.
    let tall = size_of(&r).unwrap().1;
    // The checkbox is in the "Show in export" grid of the Guide tab (the first one).
    let r = read(&rep).unwrap();
    input.click(centre(&r["tabs"][0]));
    let r = scroll_into_view(&input, &rep, "show_descriptions");
    input.click(centre(&r["show_descriptions"]));
    guide.guide_mut().unwrap().composition.show_descriptions = false;
    let want = expected(&project, &guide);
    let r = wait("the shorter layout", &|r| {
        r["preview_state"] == "ready" && size_of(r) == Some(want)
    });
    assert!(
        want.1 < tall,
        "descriptions took up room: {tall} -> {}",
        want.1
    );

    // The final-size preview draws the same layout at full size.
    input.click(centre(&r["toggle_final"]));
    let fin = wait("the final-size preview", &|r| {
        r["preview_state"] == "ready" && r["preview_final"] == true
    });
    assert_eq!(
        size_of(&fin),
        Some(want),
        "the final-size preview uses the very same layout"
    );
    assert_eq!(fin["preview_scale"].as_f64().unwrap(), 1.0);
    input.click(centre(&fin["toggle_final"]));
    wait("the quick preview again", &|r| {
        r["preview_state"] == "ready" && r["preview_final"] == false
    });

    // The settings were saved by autosave, and reopen as set.
    wait("the autosave", &|r| r["dirty"] == false);
    sleep(Duration::from_millis(300));
    let saved = ProjectStore::open(&dir).unwrap().project;
    assert_eq!(saved.guide().unwrap().composition.max_width, 800);
    assert!(!saved.guide().unwrap().composition.show_descriptions);
    assert!(
        saved.guide().unwrap().composition.show_numbers,
        "settings that were not touched stay as they were"
    );
    assert_eq!(
        expected(&project, &saved),
        want,
        "a reopened guide lays out exactly as it did"
    );

    let win = detect_backend()
        .windows()
        .unwrap_or_default()
        .into_iter()
        .find(|w: &WindowInfo| w.title == guide_core::APP_NAME)
        .expect("app window");
    input.close_window(win.id);
    wait_until("the app to quit", 15, || {
        app.0.try_wait().unwrap().map(|_| ())
    });
    let _ = fs::remove_dir_all(&tmp);
}
