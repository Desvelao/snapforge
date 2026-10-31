//! End-to-end tests of the annotation editor through the real binary: fake
//! mouse and keyboard on a virtual X server, then inspection of the saved
//! annotations and exported pixels.
//!
//! `scripts/test-editor.sh` provides Xvnc, Openbox and an xterm full of text.
//! Skipped unless SNAPFORGE_E2E=1.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread::sleep,
    time::{Duration, Instant},
};

use guide_capture::{detect_backend, CaptureMode, CaptureRequest, WindowInfo};
use guide_core::{Document, Kind, ObscureMode};
use guide_hotkeys::keysym_for;
use guide_storage::{EditBundle, OriginalImage};
use x11rb::{
    connection::Connection,
    protocol::xproto::{ClientMessageEvent, ConnectionExt as _, EventMask},
    protocol::xtest::ConnectionExt as _,
    rust_connection::RustConnection,
};

const CONTROL_L: u32 = 0xFFE3;
const SHIFT_L: u32 = 0xFFE1;

struct Input {
    conn: RustConnection,
    root: u32,
    min: u8,
    per: usize,
    keysyms: Vec<u32>,
    zoom: f64,
    origin: (f64, f64),
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
            zoom: 1.0,
            origin: (0.0, 0.0),
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

    /// Screen position of an image pixel, from the editor's own report.
    fn at(&self, ix: f64, iy: f64) -> (i16, i16) {
        (
            (self.origin.0 + ix * self.zoom).round() as i16,
            (self.origin.1 + iy * self.zoom).round() as i16,
        )
    }

    fn mv(&self, p: (i16, i16)) {
        self.fake(6, 0, p.0, p.1);
    }

    fn button_n(&self, n: u8, press: bool) {
        self.fake(if press { 4 } else { 5 }, n, 0, 0);
    }

    fn button(&self, press: bool) {
        self.button_n(1, press);
    }

    fn drag_screen(&self, button: u8, from: (i16, i16), to: (i16, i16)) {
        self.mv(from);
        sleep(Duration::from_millis(150));
        self.button_n(button, true);
        for step in 1..=8i16 {
            self.mv((
                from.0 + (to.0 - from.0) * step / 8,
                from.1 + (to.1 - from.1) * step / 8,
            ));
        }
        sleep(Duration::from_millis(150));
        self.button_n(button, false);
        sleep(Duration::from_millis(250));
    }

    fn drag_path(&self, path: &[(f64, f64)]) {
        self.mv(self.at(path[0].0, path[0].1));
        sleep(Duration::from_millis(150));
        self.button(true);
        for w in path.windows(2) {
            for step in 1..=8 {
                let t = f64::from(step) / 8.0;
                self.mv(self.at(
                    w[0].0 + (w[1].0 - w[0].0) * t,
                    w[0].1 + (w[1].1 - w[0].1) * t,
                ));
            }
        }
        sleep(Duration::from_millis(150));
        self.button(false);
        sleep(Duration::from_millis(200));
    }

    fn drag(&self, from: (f64, f64), to: (f64, f64)) {
        self.drag_path(&[from, to]);
    }

    fn click(&self, ix: f64, iy: f64) {
        self.mv(self.at(ix, iy));
        sleep(Duration::from_millis(150));
        self.button(true);
        self.button(false);
        sleep(Duration::from_millis(200));
    }

    fn wheel_up(&self, at: (i16, i16), clicks: u32) {
        self.mv(at);
        sleep(Duration::from_millis(150));
        for _ in 0..clicks {
            self.button_n(4, true);
            self.button_n(4, false);
            sleep(Duration::from_millis(80));
        }
        sleep(Duration::from_millis(400));
    }

    /// Asks the window manager to close a window, like clicking its close button.
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

fn windows() -> Vec<WindowInfo> {
    detect_backend().windows().unwrap_or_default()
}

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

type Report = serde_json::Value;

fn count(r: &Report) -> u64 {
    r["annotations"].as_u64().unwrap_or(999)
}

fn find_file(dir: &Path, suffix: &str) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.to_string_lossy().ends_with(suffix))
}

fn near(a: f32, b: f64, what: &str) {
    assert!((f64::from(a) - b).abs() <= 8.0, "{what}: {a} vs {b}");
}

/// A running app with the editor open on a full-screen capture.
struct Session {
    app: Guard,
    input: Input,
    rep: PathBuf,
    out: PathBuf,
    tmp: PathBuf,
    screen: (u32, u32),
}

impl Session {
    fn start() -> Option<Session> {
        if std::env::var_os("SNAPFORGE_E2E").is_none() {
            eprintln!("skipped: SNAPFORGE_E2E not set");
            return None;
        }
        let scale = std::env::var("SNAPFORGE_E2E_SCALE").unwrap_or_else(|_| "1".into());
        let tmp = std::env::temp_dir().join(format!(
            "snapforge-editor-e2e-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t").len()
        ));
        let _ = fs::remove_dir_all(&tmp);
        let (out, cfg, rep) = (tmp.join("out"), tmp.join("cfg"), tmp.join("report.json"));
        fs::create_dir_all(cfg.join("snapforge")).unwrap();
        fs::create_dir_all(&out).unwrap();
        fs::write(
            cfg.join("snapforge/settings.json"),
            format!("{{\"out_dir\": {:?}}}", out.to_string_lossy()),
        )
        .unwrap();
        let screen = detect_backend()
            .capture(&CaptureRequest {
                include_cursor: false,
                mode: CaptureMode::FullScreen,
            })
            .unwrap();
        let app = Guard(
            Command::new(env!("CARGO_BIN_EXE_snapforge"))
                .env("WINIT_X11_SCALE_FACTOR", &scale)
                .env("XDG_CONFIG_HOME", &cfg)
                .env("SNAPFORGE_TEST_OPEN_EDITOR", "1")
                .env("SNAPFORGE_TEST_EDITOR_REPORT", &rep)
                .spawn()
                .expect("start snapforge"),
        );
        wait_until("the app window", 30, || {
            windows()
                .into_iter()
                .find(|w| w.title == guide_core::APP_NAME)
        });
        sleep(Duration::from_millis(1500));
        let mut s = Session {
            app,
            input: Input::new(),
            rep,
            out,
            tmp,
            screen: (screen.width, screen.height),
        };
        // Capture the full screen; the test hook opens the editor on it.
        s.input.with_mods(&[CONTROL_L, SHIFT_L], "F");
        wait_until("the editor to open", 20, || {
            s.report()
                .filter(|r| r["scale"].as_f64().unwrap_or(0.0) > 0.0)
        });
        s.refresh();
        assert_eq!(count(&s.report().unwrap()), 0);
        assert!(
            s.input.zoom > 0.1 && s.input.zoom <= 1.01,
            "implausible zoom {}",
            s.input.zoom
        );
        Some(s)
    }

    fn report(&self) -> Option<Report> {
        serde_json::from_str(&fs::read_to_string(&self.rep).ok()?).ok()
    }

    fn wait_report(&self, what: &str, cond: impl Fn(&Report) -> bool) -> Report {
        wait_until(what, 20, || self.report().filter(|r| cond(r)))
    }

    fn expect(&self, n: u64, what: &str) {
        self.wait_report(what, |r| count(r) == n);
    }

    /// Lets the view settle, then reads where image pixels are on screen.
    fn refresh(&mut self) -> Report {
        sleep(Duration::from_millis(600));
        let r = self.report().expect("editor report");
        self.input.zoom = r["scale"].as_f64().unwrap();
        self.input.origin = (
            r["origin"][0].as_f64().unwrap(),
            r["origin"][1].as_f64().unwrap(),
        );
        r
    }

    fn snapshot(&self, name: &str) {
        let Some(dir) = std::env::var_os("SNAPFORGE_E2E_SHOT_DIR") else {
            return;
        };
        let shot = detect_backend()
            .capture(&CaptureRequest {
                include_cursor: false,
                mode: CaptureMode::FullScreen,
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
    }

    /// Ctrl+S, then reopens the saved folder exactly as a user would.
    fn save_and_open(&self) -> (OriginalImage, Document, PathBuf) {
        self.input.with_mods(&[CONTROL_L], "S");
        self.wait_report("the save", |r| {
            r["status"]
                .as_str()
                .is_some_and(|s| s.starts_with("Saved edit"))
                && r["dirty"] == false
        });
        sleep(Duration::from_millis(300));
        let dir = find_file(&self.out, ".guideedit").expect("a .guideedit folder was written");
        let (_, original, doc) = EditBundle::open(&dir).expect("the saved edit reopens");
        (original, doc, dir)
    }

    fn app_window(&self) -> WindowInfo {
        windows()
            .into_iter()
            .find(|w| w.title == guide_core::APP_NAME)
            .expect("app window")
    }

    fn exited(&mut self) -> bool {
        self.app.0.try_wait().unwrap().is_some()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
    }
}

#[test]
fn every_tool_undo_save_reopen_and_export_work_end_to_end() {
    let Some(mut s) = Session::start() else {
        return;
    };
    let (sw, sh) = (f64::from(s.screen.0), f64::from(s.screen.1));

    // Rectangle (R), arrow (A), pen (P), marker (M), blur (B) over the text in the xterm.
    s.input.tap("R");
    s.input.drag((900.0, 100.0), (1100.0, 200.0));
    s.expect(1, "the rectangle");
    s.input.tap("A");
    s.input.drag((900.0, 300.0), (1200.0, 400.0));
    s.expect(2, "the arrow");
    s.input.tap("P");
    s.input.drag_path(&[
        (900.0, 500.0),
        (1000.0, 560.0),
        (1100.0, 500.0),
        (1150.0, 600.0),
    ]);
    s.expect(3, "the pen stroke");
    s.input.tap("M");
    s.input.click(1300.0, 150.0);
    s.expect(4, "the marker");
    s.input.tap("B");
    s.input.drag((100.0, 120.0), (400.0, 260.0));
    s.expect(5, "the blur box");

    // Crop, then undo it; undo the blur; redo it.
    s.input.tap("C");
    s.input.drag((50.0, 50.0), (1500.0, 900.0));
    s.wait_report("the crop", |r| r["crop"] == true);
    s.input.with_mods(&[CONTROL_L], "Z");
    s.wait_report("crop undone", |r| r["crop"] == false && count(r) == 5);
    s.input.with_mods(&[CONTROL_L], "Z");
    s.expect(4, "blur undone");
    s.input.with_mods(&[CONTROL_L], "Y");
    s.expect(5, "blur redone");

    // Text: click, type "hi", finish with Ctrl+Enter.
    s.input.tap("T");
    s.input.click(900.0, 700.0);
    sleep(Duration::from_millis(500));
    s.input.tap("H");
    s.input.tap("I");
    s.input.with_mods(&[CONTROL_L], "Return");
    s.expect(6, "the text");
    s.snapshot("editor-with-annotations");

    // Select the rectangle by its outline and drag it 50 px right and down, then crop again and save.
    s.input.tap("V");
    s.input.drag((900.0, 150.0), (950.0, 200.0));
    s.input.tap("C");
    s.input.drag((50.0, 50.0), (1500.0, 900.0));
    s.wait_report("the second crop", |r| r["crop"] == true);
    s.snapshot("editor-after-crop-and-move");
    let (original, doc, bundle_dir) = s.save_and_open();

    assert_eq!(
        (f64::from(original.width), f64::from(original.height)),
        (sw, sh),
        "original is the full screenshot"
    );
    assert_eq!(doc.annotations.len(), 6);
    let kinds: Vec<_> = doc.annotations.iter().map(|a| &a.kind).collect();
    match kinds[0] {
        Kind::Rectangle { rect } => {
            near(rect.x, 950.0, "rectangle moved right by 50");
            near(rect.y, 150.0, "rectangle moved down by 50");
            near(rect.w, 200.0, "rectangle keeps its width");
            near(rect.h, 100.0, "rectangle keeps its height");
        }
        k => panic!("first annotation should be the rectangle, got {k:?}"),
    }
    match kinds[1] {
        Kind::Arrow { from, to } => {
            near(from.x, 900.0, "arrow tail x");
            near(from.y, 300.0, "arrow tail y");
            near(to.x, 1200.0, "arrow tip x");
            near(to.y, 400.0, "arrow tip y");
        }
        k => panic!("second should be the arrow, got {k:?}"),
    }
    assert!(
        matches!(kinds[2], Kind::Pen { points } if points.len() >= 4),
        "pen stroke keeps its points"
    );
    assert!(
        matches!(kinds[3], Kind::Marker { number: 1, center, .. } if (f64::from(center.x) - 1300.0).abs() <= 8.0)
    );
    let obscure_rect = match kinds[4] {
        Kind::Obscure {
            rect,
            mode: ObscureMode::Pixelate,
            strength,
        } => {
            near(rect.x, 100.0, "blur x");
            near(rect.w, 300.0, "blur width");
            assert_eq!(*strength, 14);
            *rect
        }
        k => panic!("fifth should be the blur box, got {k:?}"),
    };
    assert!(
        matches!(kinds[5], Kind::Text { text, .. } if text == "hi"),
        "typed text was saved: {:?}",
        kinds[5]
    );
    let crop = doc.crop.expect("crop is saved");

    // Export with Ctrl+E and inspect the pixels.
    s.input.with_mods(&[CONTROL_L], "E");
    let exported = wait_until("the exported image", 20, || {
        find_file(&s.out, "-edited.png")
    });
    sleep(Duration::from_millis(300));
    let img = image::open(&exported).unwrap().to_rgba8();
    assert_eq!(
        (img.width(), img.height()),
        (crop.w, crop.h),
        "export is cropped to the crop rectangle"
    );

    let at = |x: f32, y: f32| {
        *img.get_pixel(
            (x as u32).saturating_sub(crop.x),
            (y as u32).saturating_sub(crop.y),
        )
    };
    let is_red = |p: image::Rgba<u8>| p[0] > 180 && p[1] < 90 && p[2] < 90;
    let Kind::Rectangle { rect } = kinds[0] else {
        unreachable!()
    };
    assert!(
        (-2i32..=2).any(|d| is_red(at(rect.x + d as f32, rect.y + rect.h / 2.0))),
        "the rectangle outline is burned into the export"
    );

    // Redaction: every pixelation block in the export is uniform; the original there shows text.
    let (ox, oy) = (obscure_rect.x.floor() as u32, obscure_rect.y.floor() as u32);
    let block = 14u32;
    let uniform = |get: &dyn Fn(u32, u32) -> [u8; 3]| {
        (0..obscure_rect.h as u32 / block).all(|by| {
            (0..obscure_rect.w as u32 / block).all(|bx| {
                let first = get(ox + bx * block, oy + by * block);
                (0..block).all(|y| {
                    (0..block).all(|x| get(ox + bx * block + x, oy + by * block + y) == first)
                })
            })
        })
    };
    let orig = |x: u32, y: u32| {
        let i = (y as usize * original.width as usize + x as usize) * 4;
        [original.rgba[i], original.rgba[i + 1], original.rgba[i + 2]]
    };
    let exp = |x: u32, y: u32| {
        let p = img.get_pixel(x - crop.x, y - crop.y);
        [p[0], p[1], p[2]]
    };
    assert!(
        !uniform(&orig),
        "the area under the blur box must contain real detail in the original (xterm text)"
    );
    assert!(
        uniform(&exp),
        "every pixelation block in the export must be a single colour"
    );

    // Saving again must not touch the original.
    let before = fs::read(bundle_dir.join("original.png")).unwrap();
    s.input.with_mods(&[CONTROL_L], "S");
    sleep(Duration::from_millis(500));
    assert!(
        fs::read(bundle_dir.join("original.png")).unwrap() == before,
        "original.png changed"
    );

    // Closing a window with nothing unsaved quits straight away, with no prompt.
    let win = s.app_window();
    s.input.close_window(win.id);
    wait_until("the app to quit", 10, || s.exited().then_some(()));
}

#[test]
fn line_ellipse_highlighter_eraser_resize_slider_zoom_pan_and_close_prompt() {
    let Some(mut s) = Session::start() else {
        return;
    };
    let first = s.report().unwrap();
    let (scale0, origin0) = (
        first["scale"].as_f64().unwrap(),
        (
            first["origin"][0].as_f64().unwrap(),
            first["origin"][1].as_f64().unwrap(),
        ),
    );

    // Line (L), ellipse (E), highlighter (H).
    s.input.tap("L");
    s.input.drag((900.0, 100.0), (1100.0, 100.0));
    s.expect(1, "the line");
    s.input.tap("E");
    s.input.drag((900.0, 200.0), (1100.0, 300.0));
    s.expect(2, "the ellipse");
    s.input.tap("H");
    s.input.drag((100.0, 700.0), (400.0, 700.0));
    s.expect(3, "the highlighter stroke");

    // Zoom and pan: 100%, fit, Ctrl+wheel zoom, middle-button pan.
    s.input.tap("1");
    s.wait_report("100% zoom", |r| {
        (r["scale"].as_f64().unwrap_or(0.0) - 1.0).abs() < 0.01
    });
    s.input.tap("0");
    s.wait_report("fit", |r| {
        (r["scale"].as_f64().unwrap_or(0.0) - scale0).abs() < 0.002
    });
    let centre = s.input.at(960.0, 540.0);
    // Ctrl+wheel zooms; the plain wheel moves the picture instead.
    s.input.key_raw(0xffe3, true); // Control_L
    s.input.wheel_up(centre, 5);
    s.input.key_raw(0xffe3, false);
    let zoomed = s.report().unwrap()["scale"].as_f64().unwrap();
    assert!(
        zoomed > scale0 * 1.2,
        "Ctrl+wheel did not zoom in: {scale0} -> {zoomed}"
    );
    s.input.tap("0");
    s.wait_report("fit again", |r| {
        (r["scale"].as_f64().unwrap_or(0.0) - scale0).abs() < 0.002
    });
    let r = s.refresh();
    s.input.drag_screen(
        2,
        s.input.at(960.0, 300.0),
        (
            s.input.at(960.0, 300.0).0 + 40,
            s.input.at(960.0, 300.0).1 + 30,
        ),
    );
    // The report is written on the next frame, so wait for the view to settle instead of
    // reading it once (a single read raced with the last drag events on a busy machine).
    let moved = |p: &serde_json::Value| {
        (
            p["origin"][0].as_f64().unwrap_or(0.0) - r["origin"][0].as_f64().unwrap(),
            p["origin"][1].as_f64().unwrap_or(0.0) - r["origin"][1].as_f64().unwrap(),
        )
    };
    let panned = s.wait_report("the pan to finish", |p| {
        let (dx, dy) = moved(p);
        (dx - 40.0).abs() <= 3.0 && (dy - 30.0).abs() <= 3.0
    });
    let (dx, dy) = moved(&panned);
    assert!(
        (dx - 40.0).abs() <= 3.0 && (dy - 30.0).abs() <= 3.0,
        "pan moved the view by ({dx}, {dy}), expected (40, 30)"
    );
    s.input.tap("0");
    s.wait_report("fit after pan", |r| {
        (r["origin"][0].as_f64().unwrap_or(0.0) - origin0.0).abs() < 2.0
    });
    s.refresh();

    // Select the ellipse by its outline and resize it by dragging the south-east handle.
    s.input.tap("V");
    s.input.click(900.0, 250.0);
    s.wait_report("the ellipse selected", |r| {
        r["selected"].as_u64().unwrap_or(0) != 0
    });
    s.input.drag((1100.0, 300.0), (1200.0, 400.0));
    s.expect(3, "still three after the resize");

    // Drag the width slider across its rail: many changes, one undo step.
    s.snapshot("editor-ellipse-selected");
    let slider = s.report().unwrap()["slider"].clone();
    let (sx, sy, sw_, sh_) = (
        slider[0].as_f64().unwrap(),
        slider[1].as_f64().unwrap(),
        slider[2].as_f64().unwrap(),
        slider[3].as_f64().unwrap(),
    );
    let y = (sy + sh_ / 2.0) as i16;
    // The slider spans the properties panel: drag over fractions of its width.
    s.input.mv(((sx + sw_ * 0.08) as i16, y));
    sleep(Duration::from_millis(200));
    s.input.button(true);
    for f in [0.2, 0.35, 0.5] {
        s.input.mv(((sx + sw_ * f) as i16, y));
        sleep(Duration::from_millis(150));
    }
    s.input.button(false);
    sleep(Duration::from_millis(300));
    let (_, doc, _) = s.save_and_open();
    let width_after_drag = match &doc.annotations[1].kind {
        Kind::Ellipse { rect } => {
            near(rect.x, 900.0, "ellipse x");
            near(rect.w, 300.0, "ellipse resized to 300 wide by its handle");
            near(rect.h, 200.0, "ellipse resized to 200 high by its handle");
            doc.annotations[1].style.width
        }
        k => panic!("second should be the ellipse, got {k:?}"),
    };
    assert!(
        width_after_drag > 8.0,
        "the slider should have widened the outline, got {width_after_drag}"
    );
    s.input.with_mods(&[CONTROL_L], "Z");
    let (_, doc, _) = s.save_and_open();
    assert!(
        (doc.annotations[1].style.width - 4.0).abs() < 0.01,
        "one undo must undo the whole slider drag, width is {}",
        doc.annotations[1].style.width
    );
    assert!(
        matches!(&doc.annotations[1].kind, Kind::Ellipse { rect } if rect.w > 250.0),
        "the resize is a separate, earlier step"
    );

    // Eraser removes a whole annotation, and undo brings it back.
    s.input.tap("X");
    s.input.click(1000.0, 100.0);
    s.expect(2, "the line erased");
    s.input.with_mods(&[CONTROL_L], "Z");
    s.expect(3, "the erase undone");

    let (_, doc, _) = s.save_and_open();
    match &doc.annotations[0].kind {
        Kind::Line { from, to } => {
            near(from.x, 900.0, "line start x");
            near(to.x, 1100.0, "line end x");
            near(from.y, 100.0, "line y");
        }
        k => panic!("first should be the line, got {k:?}"),
    }
    match &doc.annotations[2] {
        a if matches!(a.kind, Kind::Highlighter { .. }) => {
            assert_eq!(a.style.color.0[3], 110, "highlighter is translucent");
            assert_eq!(a.style.width, 22.0);
            let Kind::Highlighter { points } = &a.kind else {
                unreachable!()
            };
            assert!(
                points.iter().all(|p| (p.y - 700.0).abs() <= 8.0),
                "{points:?}"
            );
        }
        a => panic!("third should be the highlighter, got {a:?}"),
    }

    // Closing the window with unsaved changes asks first; saving lets it quit.
    s.input.tap("A");
    s.input.drag((1300.0, 300.0), (1500.0, 400.0));
    s.wait_report("unsaved changes", |r| r["dirty"] == true && count(r) == 4);
    let win = s.app_window();
    s.input.close_window(win.id);
    s.wait_report("the unsaved-changes dialog", |r| r["confirm_close"] == true);
    sleep(Duration::from_millis(1000));
    assert!(
        !s.exited(),
        "the app must not quit while changes are unsaved"
    );
    s.snapshot("editor-unsaved-dialog");
    s.input.with_mods(&[CONTROL_L], "S");
    wait_until("the app to quit after saving", 10, || {
        s.exited().then_some(())
    });
    let (_, saved, _) = {
        let dir = find_file(&s.out, ".guideedit").unwrap();
        EditBundle::open(&dir)
            .map(|(b, o, d)| (b, d, o))
            .map(|(_, d, o)| (o.clone(), d, ()))
            .unwrap()
    };
    assert_eq!(
        saved.annotations.len(),
        4,
        "the change was saved before quitting"
    );
}
