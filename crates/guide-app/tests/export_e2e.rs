//! End-to-end test of exporting from the guide window: fake mouse and keyboard on a virtual
//! X server, then inspection of the files that were written.
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
use guide_core::{Editor, Kind, NewItem, Project, RectF, Rgba, Style};
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

    fn ctrl(&self, name: &str) {
        self.key_raw(CONTROL_L, true);
        let ks = keysym_for(name).unwrap();
        self.key_raw(ks, true);
        self.key_raw(ks, false);
        self.key_raw(CONTROL_L, false);
        sleep(Duration::from_millis(200));
    }

    fn click(&self, p: (i16, i16)) {
        self.fake(6, 0, p.0, p.1);
        sleep(Duration::from_millis(150));
        self.fake(4, 1, 0, 0);
        self.fake(5, 1, 0, 0);
        sleep(Duration::from_millis(300));
    }

    /// Scrolls the wheel down over `at` (moves a scroll area under the pointer).
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

fn centre(v: &serde_json::Value) -> (i16, i16) {
    let a = v
        .as_array()
        .unwrap_or_else(|| panic!("expected a rectangle, got {v}"));
    let f = |i: usize| a[i].as_f64().unwrap();
    ((f(0) + f(2) / 2.0) as i16, (f(1) + f(3) / 2.0) as i16)
}

/// Scrolls the properties panel until the rectangle `pick` finds in the report lies well inside the
/// app window, and returns that report. A control on the window's edge or border cannot be clicked
/// reliably; where the panel's content ends depends on the interface scale.
fn scroll_into_view(input: &Input, rep: &Path, pick: impl Fn(&Report) -> Report) -> Report {
    for _ in 0..12 {
        let r = wait_until("the control in the report", 25, || {
            read(rep).filter(|r| pick(r).is_array())
        });
        let win = detect_backend()
            .windows()
            .unwrap_or_default()
            .into_iter()
            .find(|w: &WindowInfo| w.title == guide_core::APP_NAME)
            .expect("app window")
            .rect;
        let rect = pick(&r);
        let a = rect.as_array().unwrap();
        let (top, bottom) = (
            a[1].as_f64().unwrap(),
            a[1].as_f64().unwrap() + a[3].as_f64().unwrap(),
        );
        let margin = 30.0;
        if top > f64::from(win.y) + margin && bottom < f64::from(win.y) + f64::from(win.h) - margin
        {
            // The panel scrolls smoothly: only a rectangle that holds still can be clicked.
            sleep(Duration::from_millis(400));
            match read(rep) {
                Some(again) if pick(&again) == rect => return again,
                _ => continue,
            }
        }
        // The panel is the right-hand column: wheel over the control's column, mid-window.
        let x = centre(&rect).0;
        input.wheel_down((x, (win.y + win.h as i32 / 2) as i16), 2);
    }
    panic!("could not scroll the control into view");
}

/// How long the window may take to show the effect of a click. On a loaded machine at a high
/// interface scale its frames can lag by several seconds, so a shorter wait would repeat clicks
/// that had worked (and start the export twice).
const CLICK_EFFECT_WITHIN: Duration = Duration::from_secs(12);

/// Clicks the control `pick` finds (scrolled into view first) until the report satisfies `done`.
/// `done` must describe the click's immediate effect (a mode chosen, an export started), not a
/// result that takes time such as a finished export: a click that has no visible effect within
/// a couple of seconds is repeated, and says so on stderr so a lost click shows up in the log.
fn click_until(
    input: &Input,
    rep: &Path,
    pick: impl Fn(&Report) -> Report + Copy,
    what: &str,
    done: impl Fn(&Report) -> bool,
) -> Report {
    for attempt in 1..=3 {
        let r = scroll_into_view(input, rep, pick);
        input.click(centre(&pick(&r)));
        let end = Instant::now() + CLICK_EFFECT_WITHIN;
        while Instant::now() < end {
            if let Some(r) = read(rep).filter(|r| done(r)) {
                return r;
            }
            sleep(Duration::from_millis(100));
        }
        eprintln!("click for {what} had no effect (attempt {attempt}); repeating it");
    }
    panic!(
        "clicking for {what} never took effect; last report: {}",
        read(rep).map(|r| r.to_string()).unwrap_or_default()
    );
}

fn solid(w: u32, h: u32, c: [u8; 3]) -> Vec<u8> {
    [c[0], c[1], c[2], 255].repeat(w as usize * h as usize)
}

fn files(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(d)
        .map(|r| {
            r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn px(p: &Path, x: u32, y: u32) -> [u8; 3] {
    let c = image::open(p).unwrap().to_rgba8().get_pixel(x, y).0;
    [c[0], c[1], c[2]]
}

#[test]
fn exporting_from_the_window_writes_safe_predictable_files() {
    if std::env::var_os("SNAPFORGE_E2E").is_none() {
        eprintln!("skipped: SNAPFORGE_E2E not set");
        return;
    }
    let scale = std::env::var("SNAPFORGE_E2E_SCALE").unwrap_or_else(|_| "1".into());
    let tmp = std::env::temp_dir().join(format!("guide-export-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let (dir, cfg, rep): (PathBuf, PathBuf, PathBuf) = (
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

    let mut guide = Project::new_guide("How to Change a Setting!");
    let project = ProjectStore::create(&dir, &guide).unwrap();
    let colours = [[200u8, 40, 40], [40, 200, 40], [40, 40, 200]];
    for (i, title) in ["Open settings", "Pick an option", "Save & close"]
        .iter()
        .enumerate()
    {
        let rel = project
            .add_asset_rgba(400, 300, &solid(400, 300, colours[i]))
            .unwrap();
        guide.add_item(
            usize::MAX,
            NewItem {
                title: (*title).into(),
                description: format!("Details {i}"),
                caption: String::new(),
                image: rel,
            },
        );
    }
    // Step 2 carries an annotation that must end up in the exported file.
    let mut e = Editor::new(guide_core::Document::default());
    e.add(
        Kind::Rectangle {
            rect: RectF {
                x: 40.0,
                y: 40.0,
                w: 200.0,
                h: 100.0,
            },
        },
        Style {
            color: Rgba([255, 255, 255, 255]),
            width: 8.0,
            fill: None,
        },
    );
    guide.items[1].annotations = e.document().clone();
    project.save(&guide).unwrap();
    let out = dir.join("exports");

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
                detect_backend()
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
    wait("the window", &|r| {
        r["preview_state"] == "ready" && r["export"]["button"].is_array()
    });

    // Export one image per step with the toolbar button.
    click_until(
        &input,
        &rep,
        |r| r["export"]["button"].clone(),
        "the first export to start",
        |r| r["export"]["runs"].as_u64() >= Some(1),
    );
    let r = wait("the first export", &|r| {
        r["export"]["runs"] == 1 && r["export"]["state"] == "done"
    });
    assert_eq!(r["export"]["written"], 3);
    assert_eq!(r["export"]["problems"], 0);
    assert_eq!(
        files(&out),
        [
            "01-open-settings.png",
            "02-pick-an-option.png",
            "03-save-close.png"
        ],
        "predictable, numbered, sanitized names"
    );
    for (i, c) in colours.iter().enumerate() {
        let f = out.join(files(&out)[i].clone());
        assert_eq!(image::image_dimensions(&f).unwrap(), (400, 300));
        assert_eq!(
            px(&f, 380, 280),
            *c,
            "step {} is the right screenshot, in order",
            i + 1
        );
    }
    assert_eq!(
        px(&out.join("02-pick-an-option.png"), 40, 90),
        [255, 255, 255],
        "the annotation is burned into the file"
    );
    let first_bytes: Vec<Vec<u8>> = files(&out)
        .iter()
        .map(|n| fs::read(out.join(n)).unwrap())
        .collect();

    // Exporting again never overwrites: the new files sit beside the old ones as -2.
    click_until(
        &input,
        &rep,
        |r| r["export"]["button"].clone(),
        "the second export to start",
        |r| r["export"]["runs"].as_u64() >= Some(2),
    );
    wait("the second export", &|r| {
        r["export"]["runs"] == 2 && r["export"]["state"] == "done"
    });
    assert_eq!(files(&out).len(), 6);
    assert!(files(&out).contains(&"01-open-settings-2.png".to_string()));
    for (n, b) in [
        "01-open-settings.png",
        "02-pick-an-option.png",
        "03-save-close.png",
    ]
    .iter()
    .zip(&first_bytes)
    {
        assert!(
            fs::read(out.join(n)).unwrap() == *b,
            "{n} from the first export is untouched"
        );
    }

    // Switch to the combined image and export it. The mode buttons are at the end of the
    // scrolling properties panel, so scroll them into view first.
    click_until(
        &input,
        &rep,
        |r| r["export"]["combined"].clone(),
        "the combined mode",
        |r| r["export"]["mode"] == "Combined",
    );
    click_until(
        &input,
        &rep,
        |r| r["export"]["button"].clone(),
        "the combined export to start",
        |r| r["export"]["runs"].as_u64() >= Some(3),
    );
    wait("the combined export", &|r| {
        r["export"]["runs"] == 3 && r["export"]["state"] == "done"
    });
    let combined = out.join("how-to-change-a-setting.png");
    assert!(
        combined.exists(),
        "named from the guide title: {:?}",
        files(&out)
    );
    let layout = compute_layout(&guide, &project.item_sizes(&guide)).unwrap();
    assert_eq!(
        image::image_dimensions(&combined).unwrap(),
        (layout.width, layout.height),
        "the size the layout engine predicted"
    );
    let mut last_y = -1.0;
    for (item, c) in layout.images.iter().zip(colours) {
        assert!(
            item.rect.y > last_y,
            "steps run top to bottom in guide order"
        );
        last_y = item.rect.y;
        assert_eq!(
            px(
                &combined,
                (item.rect.x + item.rect.w - 4.0) as u32,
                (item.rect.y + item.rect.h - 4.0) as u32
            ),
            c
        );
    }

    // Ctrl+E repeats the export with the same settings; it was already running in the background of the window.
    for attempt in 1..=3 {
        input.ctrl("E");
        let end = Instant::now() + CLICK_EFFECT_WITHIN;
        while Instant::now() < end && !read(&rep).is_some_and(|r| r["export"]["runs"] == 4) {
            sleep(Duration::from_millis(100));
        }
        if read(&rep).is_some_and(|r| r["export"]["runs"] == 4) {
            break;
        }
        eprintln!("Ctrl+E had no effect (attempt {attempt}); repeating it");
    }
    wait("the keyboard export", &|r| {
        r["export"]["runs"] == 4 && r["export"]["state"] == "done"
    });
    assert!(out.join("how-to-change-a-setting-2.png").exists());

    // A screenshot that has gone missing is reported, and the other steps are still exported.
    fs::remove_file(dir.join(&guide.items[2].image)).unwrap();
    click_until(
        &input,
        &rep,
        |r| r["export"]["separate"].clone(),
        "the separate mode",
        |r| r["export"]["mode"] == "Separate",
    );
    click_until(
        &input,
        &rep,
        |r| r["export"]["button"].clone(),
        "the export with a problem to start",
        |r| r["export"]["runs"].as_u64() >= Some(5),
    );
    let r = wait("the export with a problem", &|r| {
        r["export"]["runs"] == 5 && r["export"]["state"] == "done"
    });
    assert_eq!(
        r["export"]["written"], 2,
        "the two steps that could be exported were"
    );
    assert_eq!(
        r["export"]["problems"], 1,
        "and the one that could not is reported"
    );
    assert!(
        r["status"].as_str().unwrap().contains("problem"),
        "the status line says so: {}",
        r["status"]
    );
    assert!(
        files(&out).iter().all(|n| !n.ends_with(".tmp")),
        "no temp files left behind: {:?}",
        files(&out)
    );

    if let Some(shots) = std::env::var_os("SNAPFORGE_E2E_SHOT_DIR") {
        let shot = detect_backend()
            .capture(&guide_capture::CaptureRequest {
                include_cursor: false,
                mode: guide_capture::CaptureMode::FullScreen,
            })
            .unwrap();
        guide_storage::save_rgba(
            Path::new(&shots),
            "export-section",
            guide_storage::ImageFormat::Png,
            shot.width,
            shot.height,
            &shot.rgba,
        )
        .unwrap();
    }

    // Exporting never changes the guide itself.
    let after = ProjectStore::open(&dir).unwrap();
    assert_eq!(after.project.items.len(), 3);
    assert_eq!(
        after.project.items[1].annotations,
        guide.items[1].annotations
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
