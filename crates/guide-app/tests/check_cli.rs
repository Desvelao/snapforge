//! `snapforge --check`: runs the real binary on the committed format fixture. Needs no display.
use std::{path::Path, process::Command};

fn run(dir: &Path) -> (Option<i32>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_snapforge"))
        .arg("--check")
        .arg(dir)
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn the_golden_fixture_checks_out() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../guide-storage/tests/fixtures/project-guide.guideproj");
    let (code, text) = run(&dir);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("Golden guide")
            && text.contains("2 step(s), 1 included")
            && text.trim_end().ends_with("OK"),
        "{text}"
    );
}

#[test]
fn a_missing_folder_exits_with_1_and_an_explanation() {
    let (code, text) = run(Path::new("/definitely/not/a/guide"));
    assert_eq!(code, Some(1));
    assert!(text.contains("not a project"), "{text}");
}

#[test]
fn no_argument_prints_usage_and_exits_with_64() {
    let out = Command::new(env!("CARGO_BIN_EXE_snapforge"))
        .arg("--check")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
}

#[test]
fn version_and_help_need_no_display() {
    let run = |arg: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_snapforge"))
            .arg(arg)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    for flag in ["--version", "-V"] {
        let (code, text) = run(flag);
        assert_eq!(code, Some(0));
        assert!(
            text.starts_with(&format!(
                "{} {} (",
                guide_core::APP_NAME,
                env!("CARGO_PKG_VERSION")
            )),
            "{text}"
        );
    }
    let (code, text) = run("--help");
    assert_eq!(code, Some(0));
    for word in ["--check", "--self-test", "--version"] {
        assert!(text.contains(word), "help mentions {word}: {text}");
    }
}

#[test]
fn the_embedded_window_icon_is_a_valid_square_png() {
    let bytes = include_bytes!("../../../packaging/icons/hicolor/256x256/apps/snapforge.png");
    let img = image::load_from_memory(bytes).unwrap();
    assert_eq!((img.width(), img.height()), (256, 256));
}

#[test]
fn help_describes_the_check_exit_codes_the_way_the_binary_uses_them() {
    let out = Command::new(env!("CARGO_BIN_EXE_snapforge"))
        .arg("--help")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    let help = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        help.contains("1 = cannot be opened, 2 = problems found"),
        "{help}"
    );
    let (code, _) = run(Path::new("/definitely/not/a/guide"));
    assert_eq!(code, Some(1), "1 means cannot be opened");
}
