//! Runs against a real X server; skipped unless DISPLAY is set and reachable.
use guide_capture::{detect_backend, CaptureMode, CaptureRequest, Rect};

#[test]
fn x11_full_screen_and_region_capture() {
    if std::env::var_os("DISPLAY").is_none() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
        eprintln!("skipped: no X11 session");
        return;
    }
    let backend = detect_backend();
    let caps = backend.capabilities();
    if !caps.full_screen.is_yes() {
        eprintln!("skipped: {:?}", caps.full_screen);
        return;
    }
    let full = backend
        .capture(&CaptureRequest {
            include_cursor: false,
            mode: CaptureMode::FullScreen,
        })
        .expect("full screen capture");
    assert!(full.width > 0 && full.height > 0);
    assert_eq!(
        full.rgba.len(),
        full.width as usize * full.height as usize * 4
    );
    assert!(full.rgba.chunks_exact(4).all(|p| p[3] == 255));

    let region = backend
        .capture(&CaptureRequest {
            include_cursor: false,
            mode: CaptureMode::Region(Rect {
                x: 0,
                y: 0,
                w: 10,
                h: 10,
            }),
        })
        .expect("region capture");
    assert_eq!((region.width, region.height), (10, 10));

    let monitors = backend.monitors().expect("monitors");
    assert!(!monitors.is_empty());
    let m = backend
        .capture(&CaptureRequest {
            include_cursor: false,
            mode: CaptureMode::Monitor(monitors[0].id),
        })
        .expect("monitor capture");
    assert_eq!((m.width, m.height), (monitors[0].width, monitors[0].height));
}

/// Needs a running EWMH window manager with an xterm titled "snapforge-live";
/// `SNAPFORGE_EXPECT_WINDOWS=1` turns skips into failures.
#[test]
fn x11_window_listing_and_capture() {
    let strict = std::env::var_os("SNAPFORGE_EXPECT_WINDOWS").is_some();
    if std::env::var_os("DISPLAY").is_none() {
        assert!(!strict, "no DISPLAY");
        return;
    }
    let backend = detect_backend();
    if !backend.capabilities().window.is_yes() {
        assert!(
            !strict,
            "window capability: {:?}",
            backend.capabilities().window
        );
        eprintln!("skipped: no window list");
        return;
    }
    let windows = backend.windows().expect("window list");
    let w = windows
        .iter()
        .find(|w| w.title == "snapforge-live")
        .unwrap_or_else(|| panic!("xterm not listed; got {windows:?}"));
    // The frame (title bar + borders) must make the outer rect larger than 0.
    assert!(w.rect.w > 50 && w.rect.h > 50, "{w:?}");

    let shot = backend
        .capture(&CaptureRequest {
            include_cursor: false,
            mode: CaptureMode::Window(w.id),
        })
        .expect("window capture");
    let vis = guide_capture::visible_rect(w.rect, {
        let full = backend
            .capture(&CaptureRequest {
                include_cursor: false,
                mode: CaptureMode::FullScreen,
            })
            .unwrap();
        (full.width, full.height)
    })
    .unwrap();
    assert_eq!((shot.width, shot.height), (vis.w, vis.h));

    assert_eq!(
        backend.capture(&CaptureRequest {
            include_cursor: false,
            mode: CaptureMode::Window(0xdead)
        }),
        Err(guide_capture::CaptureError::NoSuchWindow(0xdead))
    );
}

/// Pointer capture: warp the pointer, capture with and without it, and check
/// that the images differ only around the pointer.
#[test]
fn x11_cursor_is_included_only_when_requested() {
    use x11rb::{connection::Connection, protocol::xproto::ConnectionExt as _};
    if std::env::var_os("DISPLAY").is_none() {
        return;
    }
    let backend = detect_backend();
    if !backend.capabilities().cursor.is_yes() {
        assert!(
            std::env::var_os("SNAPFORGE_EXPECT_WINDOWS").is_none(),
            "cursor capability: {:?}",
            backend.capabilities().cursor
        );
        return;
    }
    let (conn, screen) = x11rb::connect(None).unwrap();
    let root = conn.setup().roots[screen].root;
    conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, 300, 200)
        .unwrap();
    conn.flush().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));

    let take = |include_cursor| {
        backend
            .capture(&CaptureRequest {
                include_cursor,
                mode: CaptureMode::FullScreen,
            })
            .unwrap()
    };
    let (without, with) = (take(false), take(true));
    assert_eq!((without.width, without.height), (with.width, with.height));
    let differing: Vec<usize> = (0..without.rgba.len() / 4)
        .filter(|i| without.rgba[i * 4..i * 4 + 3] != with.rgba[i * 4..i * 4 + 3])
        .collect();
    assert!(!differing.is_empty(), "cursor was not drawn");
    for i in differing {
        let (x, y) = (
            (i % without.width as usize) as i32,
            (i / without.width as usize) as i32,
        );
        assert!(
            (x - 300).abs() <= 64 && (y - 200).abs() <= 64,
            "pixel ({x},{y}) changed far from the pointer at (300,200)"
        );
    }
}
