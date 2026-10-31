//! End-to-end test of the real binary on a virtual X server with a forced
//! UI scale factor, driven with XTEST fake keyboard and mouse input.
//!
//! Needs a window manager (Openbox) and an xterm titled "snapforge-live";
//! `scripts/test-hidpi.sh` sets that up. Skipped unless SNAPFORGE_E2E=1.
//! Checks the *pixel size of the saved files*, which is what breaks when
//! HiDPI scaling is mishandled.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread::sleep,
    time::{Duration, Instant},
};

use guide_capture::{detect_backend, visible_rect, CaptureMode, CaptureRequest, WindowInfo};
use guide_hotkeys::keysym_for;
use x11rb::{
    connection::Connection,
    protocol::{xproto::ConnectionExt as _, xtest::ConnectionExt as _},
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

    fn key(&self, keysym: u32, press: bool) {
        let kc = (self.min..=255)
            .find(|kc| self.keysyms.get(usize::from(kc - self.min) * self.per) == Some(&keysym))
            .expect("keysym in keymap");
        self.fake(if press { 2 } else { 3 }, kc, 0, 0);
    }

    fn shortcut(&self, key: &str) {
        let ks = keysym_for(key).unwrap();
        self.key(CONTROL_L, true);
        self.key(SHIFT_L, true);
        self.key(ks, true);
        self.key(ks, false);
        self.key(SHIFT_L, false);
        self.key(CONTROL_L, false);
    }

    fn move_to(&self, x: i16, y: i16) {
        self.fake(6, 0, x, y); // MotionNotify, absolute
    }

    fn button(&self, press: bool) {
        self.fake(if press { 4 } else { 5 }, 1, 0, 0);
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

fn pngs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<_> = fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    v.retain(|p| p.extension().is_some_and(|e| e == "png"));
    v.sort();
    v
}

/// Waits for one more PNG than `before` and returns its pixel size.
fn next_png_size(dir: &Path, before: usize, what: &str) -> (u32, u32) {
    let path = wait_until(what, 20, || {
        let all = pngs(dir);
        (all.len() > before).then(|| all.last().unwrap().clone())
    });
    sleep(Duration::from_millis(150)); // let the write finish
    image::image_dimensions(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn snapshot(dir: &Option<PathBuf>, name: &str) {
    let Some(dir) = dir else { return };
    let shot = detect_backend()
        .capture(&CaptureRequest {
            include_cursor: true,
            mode: CaptureMode::FullScreen,
        })
        .unwrap();
    guide_storage::save_rgba(
        dir,
        name,
        guide_storage::ImageFormat::Png,
        shot.width,
        shot.height,
        &shot.rgba,
    )
    .unwrap();
}

fn close(a: u32, b: u32, tol: u32) -> bool {
    a.abs_diff(b) <= tol
}

#[test]
fn captures_have_correct_pixel_sizes_with_ui_scaling() {
    if std::env::var_os("SNAPFORGE_E2E").is_none() {
        eprintln!("skipped: SNAPFORGE_E2E not set");
        return;
    }
    let scale: f32 = std::env::var("SNAPFORGE_E2E_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2.0);
    let tmp = std::env::temp_dir().join(format!("snapforge-e2e-{}", std::process::id()));
    let (out, cfg) = (tmp.join("out"), tmp.join("cfg"));
    fs::create_dir_all(&out).unwrap();
    let shots = std::env::var_os("SNAPFORGE_E2E_SHOT_DIR").map(PathBuf::from);

    let screen = detect_backend()
        .capture(&CaptureRequest {
            include_cursor: false,
            mode: CaptureMode::FullScreen,
        })
        .unwrap();
    let (sw, sh) = (screen.width, screen.height);

    let mut app = Guard(
        Command::new(env!("CARGO_BIN_EXE_snapforge"))
            .env("WINIT_X11_SCALE_FACTOR", scale.to_string())
            .env("XDG_CONFIG_HOME", &cfg)
            .env("SNAPFORGE_TEST_AUTOSAVE_DIR", &out)
            .spawn()
            .expect("start snapforge"),
    );
    let app_window = wait_until("the app window", 30, || {
        windows()
            .into_iter()
            .find(|w| w.title == guide_core::APP_NAME)
    });
    sleep(Duration::from_millis(1500)); // first frames, hotkey registration
    snapshot(&shots, &format!("1-start-scale{scale}"));

    // The window must be ~1100 logical points wide whatever the scale (main.rs sets 1100x720 points).
    let logical_w = app_window.rect.w as f32 / scale;
    assert!(
        (1080.0..=1120.0).contains(&logical_w),
        "window is {}px wide = {logical_w} points at scale {scale}; scaling was not applied",
        app_window.rect.w
    );

    let input = Input::new();

    // 1. Full screen via global shortcut: must be physical pixels.
    input.shortcut("F");
    let size = next_png_size(&out, 0, "the full-screen capture");
    assert_eq!(size, (sw, sh), "full-screen capture size");
    wait_until("the window to come back", 15, || {
        windows()
            .into_iter()
            .find(|w| w.title == guide_core::APP_NAME)
    });
    sleep(Duration::from_millis(800));
    snapshot(&shots, &format!("2-after-fullscreen-scale{scale}"));

    if std::env::var_os("SNAPFORGE_E2E_FULLSCREEN_ONLY").is_some() {
        // The preview texture of a huge desktop must not take the app down.
        sleep(Duration::from_secs(2));
        assert!(
            app.0.try_wait().unwrap().is_none(),
            "the app exited after capturing a {sw}x{sh} desktop"
        );
        assert!(
            windows().iter().any(|w| w.title == guide_core::APP_NAME),
            "window gone"
        );
        let _ = fs::remove_dir_all(&tmp);
        return;
    }

    // 2. Region: drag 600x500 physical pixels on the frozen frame.
    input.shortcut("R");
    wait_until("the region overlay", 20, || {
        windows()
            .into_iter()
            .find(|w| w.title == guide_core::APP_NAME && w.rect.w >= sw && w.rect.h >= sh)
    });
    sleep(Duration::from_millis(1200));
    input.move_to(400, 300);
    input.button(true);
    for step in 1..=12i16 {
        input.move_to(400 + 600 * step / 12, 300 + 500 * step / 12);
    }
    sleep(Duration::from_millis(300));
    snapshot(&shots, &format!("3-region-drag-scale{scale}"));
    input.button(false);
    let (w, h) = next_png_size(&out, 1, "the region capture");
    assert!(
        close(w, 600, 2) && close(h, 500, 2),
        "region capture was {w}x{h}, expected about 600x500"
    );

    // 3. Window: hover and click the xterm.
    wait_until("the window to come back", 15, || {
        windows()
            .into_iter()
            .find(|w| w.title == guide_core::APP_NAME)
    });
    sleep(Duration::from_millis(800));
    let xterm = windows()
        .into_iter()
        .find(|w| w.title == "snapforge-live")
        .expect("xterm window");
    let expected = visible_rect(xterm.rect, (sw, sh)).unwrap();
    input.shortcut("W");
    wait_until("the window picker", 20, || {
        windows()
            .into_iter()
            .find(|w| w.title == guide_core::APP_NAME && w.rect.w >= sw && w.rect.h >= sh)
    });
    sleep(Duration::from_millis(1200));
    let (cx, cy) = (expected.x + expected.w / 2, expected.y + expected.h / 2);
    input.move_to(cx as i16, cy as i16);
    sleep(Duration::from_millis(500));
    input.move_to(cx as i16 + 3, cy as i16 + 3);
    sleep(Duration::from_millis(500));
    snapshot(&shots, &format!("4-window-hover-scale{scale}"));
    input.button(true);
    input.button(false);
    let (w, h) = next_png_size(&out, 2, "the window capture");
    assert_eq!((w, h), (expected.w, expected.h), "window capture size");

    let _ = fs::remove_dir_all(&tmp);
}
