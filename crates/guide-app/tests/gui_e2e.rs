//! End-to-end tests of `snapforge gui` (flameshot-style one-shot capture) through the real binary:
//! frozen overlay, region drag, editor, Enter / Ctrl+S / Esc, and the clipboard that must survive
//! the exit. `scripts/test-guide.sh` provides Xvnc and Openbox. The display tests are skipped
//! unless SNAPFORGE_E2E=1.
mod common;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread::sleep,
    time::{Duration, Instant},
};

use common::*;
use guide_capture::{detect_backend, WindowInfo};

const CONTROL_L: u32 = 0xFFE3;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_snapforge"))
}

/// Starts `snapforge gui <args>` with its own settings folder.
fn start(tag: &str, args: &[&str]) -> (Guard, PathBuf) {
    let scale = std::env::var("SNAPFORGE_E2E_SCALE").unwrap_or_else(|_| "1".into());
    let tmp = std::env::temp_dir().join(format!("snapforge-gui-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let cfg = tmp.join("cfg");
    fs::create_dir_all(cfg.join("snapforge")).unwrap();
    fs::write(
        cfg.join("snapforge/settings.json"),
        format!("{{\"out_dir\": {:?}}}", tmp.join("settings-out")),
    )
    .unwrap();
    let child = bin()
        .arg("gui")
        .args(args)
        .env("WINIT_X11_SCALE_FACTOR", scale)
        .env("XDG_CONFIG_HOME", &cfg)
        .spawn()
        .expect("start snapforge gui");
    (Guard(child), tmp)
}

fn windows() -> Vec<WindowInfo> {
    detect_backend()
        .windows()
        .unwrap_or_default()
        .into_iter()
        .filter(|w| w.title == guide_core::APP_NAME)
        .collect()
}

fn exit_code(app: &mut Guard, secs: u64) -> Option<i32> {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Some(status) = app.0.try_wait().unwrap() {
            return status.code();
        }
        sleep(Duration::from_millis(100));
    }
    None
}

/// Processes running the hidden clipboard server.
fn clipboard_servers() -> usize {
    fs::read_dir("/proc")
        .unwrap()
        .filter_map(|e| fs::read(e.ok()?.path().join("cmdline")).ok())
        .filter(|c| String::from_utf8_lossy(c).contains("--serve-clipboard"))
        .count()
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|d| d.filter_map(|e| Some(e.ok()?.path())).collect())
        .unwrap_or_default()
}

/// Waits for the overlay, drags a `w`x`h` region on it and waits for the editor window.
fn select_region(input: &Input, w: i16, h: i16) -> WindowInfo {
    // The window is first mapped at its default size and only then made fullscreen, so wait
    // until it really covers the screen.
    let screen_width = screen().width as i32;
    let overlay = wait_until("the fullscreen overlay", 20, || {
        windows()
            .into_iter()
            .find(|w| w.rect.w as i32 >= screen_width - 10)
    });
    assert_eq!(windows().len(), 1, "no main window next to the overlay");
    sleep(Duration::from_millis(1200));
    keep("gui-overlay", &screen());
    let (x, y) = (300, 300);
    // Fake input can outrun the toolkit's pointer tracking, so a lost drag is retried (and logged).
    let mut editor = None;
    for attempt in 1..=3 {
        input.drag((x, y), (x + w, y + h));
        let end = Instant::now() + Duration::from_secs(8);
        while editor.is_none() && Instant::now() < end {
            // The overlay is fullscreen; the editor is a normal, smaller window.
            editor = windows()
                .into_iter()
                .find(|win| win.rect.w < overlay.rect.w - 100 && win.rect.w > 300);
            sleep(Duration::from_millis(100));
        }
        if editor.is_some() {
            break;
        }
        eprintln!("note: drag attempt {attempt} did not open the editor; retrying");
    }
    if editor.is_none() {
        keep("gui-stuck", &screen());
        eprintln!("windows: {:?}", windows());
    }
    let editor = editor.expect("the editor never opened after three drags");
    sleep(Duration::from_millis(1000));
    keep("gui-editor", &screen());
    editor
}

fn click_canvas(input: &Input, win: &WindowInfo) {
    // Gives the window keyboard focus without changing anything (the Select tool is active).
    input.click((
        (win.rect.x + win.rect.w as i32 / 2) as i16,
        (win.rect.y + win.rect.h as i32 / 2) as i16,
    ));
}

#[test]
fn enter_copies_the_region_and_the_clipboard_outlives_the_program() {
    if !enabled() {
        return;
    }
    let (mut app, tmp) = start("copy", &[]);
    let input = Input::new();
    let editor = select_region(&input, 400, 300);
    click_canvas(&input, &editor);
    input.tap("Return");
    assert_eq!(
        exit_code(&mut app, 20),
        Some(0),
        "the program exits after Enter"
    );

    // The program is gone; the clipboard must still hold exactly the dragged region.
    let mut clip = arboard::Clipboard::new().unwrap();
    let img = clip.get_image().expect("an image on the clipboard");
    assert!(
        (img.width as i32 - 400).abs() <= 2 && (img.height as i32 - 300).abs() <= 2,
        "clipboard image is {}x{}, expected about 400x300",
        img.width,
        img.height
    );
    assert_eq!(clipboard_servers(), 1, "one background clipboard server");
    assert!(
        files_in(&tmp.join("settings-out")).is_empty(),
        "copying saves no file"
    );

    // Copying something else replaces the clipboard and the server goes away.
    clip.set_text("something else").unwrap();
    wait_until("the clipboard server to exit", 10, || {
        (clipboard_servers() == 0).then_some(())
    });
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn ctrl_s_saves_the_annotated_region_into_the_path_and_exits() {
    if !enabled() {
        return;
    }
    let before = screen();
    let corner = before.rgba[..4].to_vec();
    let uniform = before.rgba.chunks_exact(4).all(|p| p[..3] == corner[..3]);
    assert!(
        uniform,
        "the test needs a plain desktop to find the image on screen"
    );
    let out = std::env::temp_dir().join(format!("snapforge-gui-out-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    let (mut app, tmp) = start("save", &["--path", out.to_str().unwrap()]);
    let input = Input::new();
    let editor = select_region(&input, 400, 300);
    click_canvas(&input, &editor);
    input.tap("r"); // rectangle tool
                    // Find the image on screen: the capture is uniform, so it is the bounding box of pixels of
                    // its colour inside the editor window (the editor's own background is a different grey).
    let shot = screen();
    let r = &editor.rect;
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for y in r.y.max(0)..(r.y + r.h as i32).min(shot.height as i32) {
        for x in r.x.max(0)..(r.x + r.w as i32).min(shot.width as i32) {
            let i = (y as usize * shot.width as usize + x as usize) * 4;
            if shot.rgba[i..i + 3] == corner[..3] {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    assert!(x1 > x0 && y1 > y0, "the image was not found on screen");
    let (cx, cy) = (((x0 + x1) / 2) as i16, ((y0 + y1) / 2) as i16);
    // Centred in the window: the editor fitted the image after the window settled.
    let (wx, wy) = (r.x + r.w as i32 / 2, r.y + r.h as i32 / 2);
    assert!(
        (cx as i32 - wx).abs() < 200 && (cy as i32 - wy).abs() < 250,
        "image centre ({cx},{cy}) is far from the window centre ({wx},{wy})"
    );
    input.drag((cx - 60, cy - 40), (cx + 60, cy + 40));
    sleep(Duration::from_millis(500));
    keep("gui-annotated", &screen());
    input.with_mods(&[CONTROL_L], "s");
    assert_eq!(
        exit_code(&mut app, 20),
        Some(0),
        "the program exits after Ctrl+S"
    );

    let files = files_in(&out);
    assert_eq!(files.len(), 1, "exactly one image in --path: {files:?}");
    let saved = image::open(&files[0]).unwrap().into_rgba8();
    assert!(
        (saved.width() as i32 - 400).abs() <= 2 && (saved.height() as i32 - 300).abs() <= 2,
        "saved {}x{}",
        saved.width(),
        saved.height()
    );
    {
        let marked = saved.pixels().filter(|p| p.0[..3] != corner[..3]).count();
        assert!(
            marked > 100,
            "the rectangle is in the saved image ({marked} marked pixels)"
        );
    }
    assert!(
        files_in(&tmp.join("settings-out")).is_empty(),
        "the saved settings folder was not used"
    );
    let _ = fs::remove_dir_all(&out);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn escape_cancels_in_the_overlay_and_in_the_editor_without_side_effects() {
    if !enabled() {
        return;
    }
    let out = std::env::temp_dir().join(format!("snapforge-gui-cancel-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    let servers_before = clipboard_servers();
    let input = Input::new();

    let (mut app, _tmp) = start("esc1", &["--path", out.to_str().unwrap()]);
    wait_until("the overlay", 20, || windows().into_iter().next());
    sleep(Duration::from_millis(1200));
    input.tap("Escape");
    assert_eq!(
        exit_code(&mut app, 15),
        Some(0),
        "Escape in the overlay exits 0"
    );

    let (mut app, _tmp) = start("esc2", &["--path", out.to_str().unwrap()]);
    let editor = select_region(&input, 300, 200);
    click_canvas(&input, &editor);
    input.tap("Escape");
    sleep(Duration::from_millis(400));
    input.tap("Escape");
    assert_eq!(
        exit_code(&mut app, 15),
        Some(0),
        "Escape in the editor exits 0"
    );

    assert!(files_in(&out).is_empty(), "cancelling saves nothing");
    assert_eq!(clipboard_servers(), servers_before, "and copies nothing");
}

#[test]
fn the_delay_postpones_the_capture() {
    if !enabled() {
        return;
    }
    let started = Instant::now();
    let (mut app, _tmp) = start("delay", &["--delay", "2500"]);
    wait_until("the overlay", 20, || windows().into_iter().next());
    assert!(
        started.elapsed() >= Duration::from_millis(2400),
        "the overlay appeared after {:?}, before the 2.5 s delay",
        started.elapsed()
    );
    sleep(Duration::from_millis(800));
    Input::new().tap("Escape");
    assert_eq!(exit_code(&mut app, 15), Some(0));
}

fn run_without_display(args: &[&str]) -> (Option<i32>, String) {
    let out = bin()
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn without_a_display_the_command_fails_with_a_message_and_exit_1() {
    let (code, err) = run_without_display(&["gui"]);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("capture is not available"), "{err}");
}

#[test]
fn bad_arguments_exit_64_with_the_usage() {
    let (code, err) = run_without_display(&["gui", "--delay", "soon"]);
    assert_eq!(code, Some(64), "{err}");
    assert!(err.contains("usage: snapforge gui"), "{err}");
    let (code, _) = run_without_display(&["gui", "--bogus"]);
    assert_eq!(code, Some(64));
}

#[test]
fn the_shortcut_help_is_shown_until_f1_hides_it_and_escape_still_cancels() {
    if !enabled() {
        return;
    }
    let (mut app, tmp) = start("help", &[]);
    let input = Input::new();
    let editor = select_region(&input, 900, 560);
    click_canvas(&input, &editor);
    sleep(Duration::from_millis(600));
    let shown = screen();
    keep("gui-help-shown", &shown);

    input.tap("F1");
    sleep(Duration::from_millis(600));
    let hidden = screen();
    assert!(
        differing_pixels(&shown, &hidden) > 2000,
        "F1 should hide the help in the corner"
    );

    input.tap("F1");
    sleep(Duration::from_millis(600));
    assert!(
        differing_pixels(&hidden, &screen()) > 2000,
        "F1 should show the help again"
    );

    input.tap("Escape");
    assert_eq!(exit_code(&mut app, 20), Some(0), "Escape still cancels");
    assert!(files_in(&tmp.join("settings-out")).is_empty());
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn the_shortcut_help_is_on_the_selection_screen_and_f1_hides_it() {
    if !enabled() {
        return;
    }
    let (mut app, tmp) = start("overlay-help", &[]);
    let screen_width = screen().width as i32;
    wait_until("the fullscreen overlay", 20, || {
        windows()
            .into_iter()
            .find(|w| w.rect.w as i32 >= screen_width - 10)
    });
    sleep(Duration::from_millis(1500));
    let shown = screen();
    keep("gui-overlay-help-shown", &shown);

    let input = Input::new();
    input.tap("F1");
    sleep(Duration::from_millis(800));
    let hidden = screen();
    assert!(
        differing_pixels(&shown, &hidden) > 2000,
        "F1 should hide the help on the selection screen"
    );

    input.tap("Escape");
    assert_eq!(exit_code(&mut app, 20), Some(0), "Escape cancels");
    let _ = fs::remove_dir_all(&tmp);
}

/// Polls the mapped app windows every few milliseconds until `done` says to stop (or
/// `secs` pass) and returns the widths of every window seen, in the order first seen.
fn widths_seen(secs: u64, mut done: impl FnMut(&[u32]) -> bool) -> Vec<u32> {
    let mut seen: Vec<u32> = Vec::new();
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        for w in windows() {
            if seen.last() != Some(&w.rect.w) {
                seen.push(w.rect.w);
            }
        }
        if done(&seen) {
            break;
        }
        sleep(Duration::from_millis(4));
    }
    seen
}

#[test]
fn the_window_is_never_seen_smaller_than_the_screen_while_it_starts() {
    if !enabled() {
        return;
    }
    let screen_width = screen().width;
    let (mut app, tmp) = start("no-start-flash", &[]);
    let seen = widths_seen(20, |seen| seen.iter().any(|w| *w + 10 >= screen_width));
    assert!(
        seen.iter().any(|w| *w + 10 >= screen_width),
        "the overlay never became fullscreen: {seen:?}"
    );
    assert!(
        seen.iter().all(|w| *w + 10 >= screen_width),
        "a smaller window was visible while starting: {seen:?}"
    );
    Input::new().tap("Escape");
    exit_code(&mut app, 20);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn escape_on_the_selection_screen_closes_it_without_a_smaller_window_showing_first() {
    if !enabled() {
        return;
    }
    let screen_width = screen().width;
    let (mut app, tmp) = start("no-exit-flash", &[]);
    widths_seen(20, |seen| seen.iter().any(|w| *w + 10 >= screen_width));
    sleep(Duration::from_millis(1200));
    Input::new().tap("Escape");
    let seen = widths_seen(5, |_| windows().is_empty());
    assert!(
        seen.iter().all(|w| *w + 10 >= screen_width),
        "a smaller window showed while closing: {seen:?}"
    );
    assert_eq!(exit_code(&mut app, 20), Some(0));
    let _ = fs::remove_dir_all(&tmp);
}
