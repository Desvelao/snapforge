//! Shared helpers of the X11 end-to-end tests (fake input, screenshots, launching the app).
#![allow(dead_code)]
use std::{
    fs,
    path::Path,
    process::{Child, Command},
    thread::sleep,
    time::{Duration, Instant},
};

use guide_capture::{detect_backend, Screenshot};
use guide_hotkeys::keysym_for;
use x11rb::{
    connection::Connection,
    protocol::{xproto::ConnectionExt as _, xtest::ConnectionExt as _},
    rust_connection::RustConnection,
};

pub const SHIFT_L: u32 = 0xFFE1;

pub struct Input {
    pub conn: RustConnection,
    pub root: u32,
    pub min: u8,
    pub per: usize,
    pub keysyms: Vec<u32>,
}

impl Input {
    pub fn new() -> Self {
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

    pub fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) {
        self.conn
            .xtest_fake_input(kind, detail, 0, self.root, x, y, 0)
            .unwrap();
        self.conn.flush().unwrap();
        sleep(Duration::from_millis(25));
    }

    pub fn key_raw(&self, keysym: u32, press: bool) {
        let kc = (self.min..=255)
            .find(|kc| self.keysyms.get(usize::from(kc - self.min) * self.per) == Some(&keysym))
            .expect("keysym in keymap");
        self.fake(if press { 2 } else { 3 }, kc, 0, 0);
    }

    pub fn tap(&self, name: &str) {
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

    pub fn with_mods(&self, mods: &[u32], name: &str) {
        for m in mods {
            self.key_raw(*m, true);
        }
        self.tap(name);
        for m in mods.iter().rev() {
            self.key_raw(*m, false);
        }
        sleep(Duration::from_millis(150));
    }

    pub fn mv(&self, p: (i16, i16)) {
        self.fake(6, 0, p.0, p.1);
    }

    pub fn drag(&self, from: (i16, i16), to: (i16, i16)) {
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

    pub fn click(&self, p: (i16, i16)) {
        self.mv(p);
        sleep(Duration::from_millis(150));
        self.fake(4, 1, 0, 0);
        self.fake(5, 1, 0, 0);
        sleep(Duration::from_millis(250));
    }
}

pub struct Guard(pub Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn wait_until<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Some(v) = f() {
            return v;
        }
        sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

pub fn screen() -> Screenshot {
    detect_backend()
        .capture(&guide_capture::CaptureRequest {
            include_cursor: false,
            mode: guide_capture::CaptureMode::FullScreen,
        })
        .unwrap()
}

pub fn keep(name: &str, shot: &Screenshot) {
    if let Some(dir) = std::env::var_os("SNAPFORGE_E2E_SHOT_DIR") {
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
}

/// Mean brightness (0-255) of the window's area.
pub fn brightness(shot: &Screenshot, r: &guide_capture::IRect) -> f64 {
    let (mut sum, mut n) = (0f64, 0f64);
    for y in r.y.max(0)..(r.y + r.h as i32).min(shot.height as i32) {
        for x in r.x.max(0)..(r.x + r.w as i32).min(shot.width as i32) {
            let i = (y as usize * shot.width as usize + x as usize) * 4;
            let p = &shot.rgba[i..i + 3];
            sum += (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) as f64 / 3.0;
            n += 1.0;
        }
    }
    sum / n.max(1.0)
}

pub fn differing_pixels(a: &Screenshot, b: &Screenshot) -> usize {
    a.rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .filter(|(p, q)| p != q)
        .count()
}

/// Starts the app with `settings` (JSON) and `env`, and returns it with its window.
pub fn launch(
    tag: &str,
    settings: &str,
    env: &[(&str, &str)],
) -> (Guard, guide_capture::WindowInfo) {
    let scale = std::env::var("SNAPFORGE_E2E_SCALE").unwrap_or_else(|_| "1".into());
    let tmp = std::env::temp_dir().join(format!("snapforge-a11y-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let cfg = tmp.join("cfg");
    fs::create_dir_all(cfg.join("snapforge")).unwrap();
    fs::write(
        cfg.join("snapforge/settings.json"),
        settings.replace("OUT", &tmp.to_string_lossy()),
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snapforge"));
    cmd.env("WINIT_X11_SCALE_FACTOR", &scale)
        .env("XDG_CONFIG_HOME", &cfg);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let app = Guard(cmd.spawn().expect("start snapforge"));
    let win = wait_until("the window", 20, || {
        detect_backend()
            .windows()
            .unwrap_or_default()
            .into_iter()
            .find(|w| w.title == guide_core::APP_NAME)
    });
    sleep(Duration::from_millis(1200));
    (app, win)
}

pub fn enabled() -> bool {
    if std::env::var_os("SNAPFORGE_E2E").is_none() {
        eprintln!("skipped: SNAPFORGE_E2E not set");
        return false;
    }
    true
}
