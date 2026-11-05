mod app;
mod check;
mod clicks;
mod clipboard;
mod dialog;
mod editor;
mod gui;
mod i18n;
mod notify;
mod profile_ui;
mod project;
mod theme;

use guide_core::{APP_NAME, APP_VERSION};

/// Headless check used by `make portable-check` and `make test-x11`: takes
/// one full-screen capture, loads the saved settings and registers the global
/// shortcuts, without opening a window. Exits non-zero if any step fails.
fn self_test() -> std::process::ExitCode {
    use guide_capture::{detect_backend, CaptureMode, CaptureRequest};
    use guide_storage::Settings;

    let mut ok = true;
    let backend = detect_backend();
    println!("{APP_NAME} {APP_VERSION}");
    println!("backend: {}", backend.capabilities().backend);
    match backend.capture(&CaptureRequest {
        include_cursor: false,
        mode: CaptureMode::FullScreen,
    }) {
        Ok(shot) => println!("captured {}x{}", shot.width, shot.height),
        Err(e) => {
            eprintln!("capture failed: {e}");
            ok = false;
        }
    }

    let loaded = match Settings::default_path() {
        Ok(path) => {
            println!("settings file: {}", path.display());
            Settings::load(&path)
        }
        Err(e) => {
            eprintln!("settings: {e}");
            ok = false;
            guide_storage::Loaded {
                settings: Settings::default(),
                notice: None,
            }
        }
    };
    if let Some(notice) = &loaded.notice {
        println!("settings notice: {notice}");
    }
    println!(
        "settings: format={:?} delay={}s cursor={} mode={:?} dir={}",
        loaded.settings.format,
        loaded.settings.delay_secs,
        loaded.settings.include_cursor,
        loaded.settings.mode,
        loaded.settings.out_dir
    );

    let mut hotkeys = guide_hotkeys::detect(|| {});
    let bindings = loaded.settings.shortcuts.bindings();
    for ((action, shortcut), (_, result)) in bindings.iter().zip(hotkeys.register(&bindings)) {
        match result {
            Ok(()) => println!("shortcut {} = {shortcut}: registered", action.label()),
            Err(e) => {
                println!("shortcut {} = {shortcut}: FAILED ({e})", action.label());
                ok = false;
            }
        }
    }
    if ok {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

const HELP: &str = "\
capture screenshots, annotate them and organise them into projects and step-by-step guides.

Usage:
  snapforge                     open the application
  snapforge --check <folder>    check a project folder without a display
                                (exit 0 = fine, 1 = cannot be opened, 2 = problems found)
  snapforge gui [--delay MS] [--path DIR]
                                freeze the screen, drag a region, annotate, Enter copies it
                                to the clipboard, Ctrl+S saves it, Esc cancels; opens no
                                main window (bind it to a keyboard shortcut)
  snapforge clicks [--count N] [--path DIR]
                                save a screenshot of every left click (a full-screen capture,
                                no window) until N shots were taken or Ctrl+C
  snapforge --self-test         headless check of capture, settings and shortcuts
  snapforge --version           print the version
  snapforge --help              print this text

Documentation: docs/ in the release archive (install.md, troubleshooting.md).
";

/// The window and taskbar icon (the 256 px PNG of packaging/icons).
fn window_icon() -> Option<eframe::egui::IconData> {
    let img = image::load_from_memory(include_bytes!(
        "../../../packaging/icons/hicolor/256x256/apps/snapforge.png"
    ))
    .ok()?
    .into_rgba8();
    let (width, height) = img.dimensions();
    Some(eframe::egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

/// `snapforge gui`: capture before any window exists, then show the selection overlay.
fn run_gui(args: &[String]) -> eframe::Result {
    use guide_capture::{detect_backend, CaptureMode, CaptureRequest, Support};
    let options = match gui::parse_gui_args(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("snapforge gui: {e}\n{}", gui::GUI_USAGE);
            std::process::exit(64);
        }
    };
    let settings = guide_storage::Settings::default_path()
        .ok()
        .map(|p| guide_storage::Settings::load(&p).settings)
        .unwrap_or_default();
    std::thread::sleep(options.delay);
    let backend = detect_backend();
    let caps = backend.capabilities();
    if let Support::No(why) = &caps.full_screen {
        eprintln!("snapforge gui: capture is not available: {why}");
        std::process::exit(1);
    }
    let shot = match backend.capture(&CaptureRequest {
        include_cursor: settings.include_cursor && caps.cursor.is_yes(),
        mode: CaptureMode::FullScreen,
    }) {
        Ok(shot) => shot,
        Err(e) => {
            eprintln!("snapforge gui: capture failed: {e}");
            std::process::exit(1);
        }
    };
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_fullscreen(true)
        .with_decorations(false)
        .with_min_inner_size([640.0, 420.0])
        .with_app_id("snapforge");
    // The window manager only makes the window fullscreen after it is shown; until then it
    // would be a default-size window. Opening it where the fullscreen one will be avoids that.
    let monitors = backend.monitors().unwrap_or_default();
    if let Some(m) = gui::overlay_monitor(&monitors, backend.pointer_position()) {
        viewport = viewport
            .with_position([m.x as f32, m.y as f32])
            .with_inner_size([m.width as f32, m.height as f32]);
    }
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let result = eframe::run_native(
        APP_NAME,
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(app::MainApp::new_gui(
                cc.egui_ctx.clone(),
                shot,
                options,
            )))
        }),
    );
    if let Err(e) = &result {
        eprintln!("{APP_NAME} could not open its window: {e}");
    }
    result
}

fn main() -> eframe::Result {
    match std::env::args().nth(1).as_deref() {
        Some("gui") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return run_gui(&rest);
        }
        Some("clicks") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            std::process::exit(clicks::run(&rest));
        }
        Some("--serve-clipboard") => {
            let Some(png) = std::env::args().nth(2) else {
                std::process::exit(64);
            };
            // Hidden mode started by the program itself; see clipboard.rs.
            std::process::exit(match clipboard::serve(std::path::Path::new(&png)) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("clipboard: {e}");
                    1
                }
            });
        }
        Some("--version" | "-V") => {
            println!("{APP_NAME} {} ({})", APP_VERSION, std::env::consts::ARCH);
            std::process::exit(0);
        }
        Some("--help" | "-h") => {
            print!("{APP_NAME} {APP_VERSION} - {HELP}");
            std::process::exit(0);
        }
        _ => {}
    }
    if std::env::args().nth(1).as_deref() == Some("--check") {
        let Some(dir) = std::env::args().nth(2) else {
            eprintln!("usage: snapforge --check <project folder>");
            std::process::exit(64);
        };
        let (code, text) = check::run(std::path::Path::new(&dir));
        print!("{text}");
        std::process::exit(code);
    }
    if std::env::args().nth(1).as_deref() == Some("--self-test") {
        std::process::exit(if self_test() == std::process::ExitCode::SUCCESS {
            0
        } else {
            1
        });
    }
    let result = eframe::run_native(
        APP_NAME,
        eframe::NativeOptions {
            // Sizes are in logical points, so the window is the same physical
            // size on a 2x screen as a 1x one. winit's own default (800x600)
            // is in physical pixels and would open half-size on HiDPI.
            viewport: {
                let mut v = eframe::egui::ViewportBuilder::default()
                    .with_inner_size([1100.0, 720.0])
                    .with_min_inner_size([640.0, 420.0])
                    // Matches StartupWMClass in packaging/snapforge.desktop.
                    .with_app_id("snapforge");
                if let Some(icon) = window_icon() {
                    v = v.with_icon(icon);
                }
                v
            },
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(app::MainApp::new(cc.egui_ctx.clone())))),
    );
    if let Err(e) = &result {
        eprintln!(
            "{APP_NAME} could not open its window: {e}\n\
             This usually means a desktop runtime library is missing. \
             See docs/platform-support.md for the list."
        );
    }
    result
}
