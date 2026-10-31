//! Needs its own X server because it changes the screen size, which would disturb tests running
//! beside it (`scripts/test-x11.sh` runs it alone). Skipped unless DISPLAY is set and reachable.
use std::process::Command;

use guide_capture::{detect_backend, CaptureMode, CaptureRequest};

#[test]
fn a_screen_resize_while_running_is_captured_at_the_new_size() {
    if std::env::var_os("DISPLAY").is_none() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
        eprintln!("skipped: no X11 session");
        return;
    }
    let backend = detect_backend();
    if !backend.capabilities().full_screen.is_yes() {
        eprintln!("skipped: full screen capture unavailable");
        return;
    }
    let request = CaptureRequest {
        include_cursor: false,
        mode: CaptureMode::FullScreen,
    };
    let before = backend.capture(&request).expect("capture before");
    let (w, h) = (800u16, 600u16);
    // Xvfb has one output ("screen") with one mode; add a smaller one and switch to it.
    let steps = [
        "--newmode 800x600_t 25.00 800 816 880 960 600 601 604 625 -hsync +vsync",
        "--addmode screen 800x600_t",
        "--output screen --mode 800x600_t",
    ];
    for args in steps {
        let ok = Command::new("xrandr").args(args.split(' ')).status();
        if !matches!(ok, Ok(s) if s.success()) {
            eprintln!("skipped: xrandr cannot change the mode of this server");
            return;
        }
    }
    assert!(before.width != w as u32, "the test needs a different size");
    let after = backend.capture(&request).expect("capture after resize");
    assert_eq!((after.width, after.height), (w as u32, h as u32));
    assert_eq!(
        after.rgba.len(),
        after.width as usize * after.height as usize * 4
    );
}
