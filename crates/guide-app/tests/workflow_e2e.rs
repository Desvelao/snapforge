//! End-to-end check of the first-run workflow through the real binary: capture from the keyboard,
//! the post-capture routes, the shortcut help, and a second capture.
//!
//! `scripts/test-guide.sh` provides Xvnc and Openbox. Skipped unless SNAPFORGE_E2E=1.
mod common;
use std::{fs, thread::sleep, time::Duration};

use common::*;

const CONTROL_L: u32 = 0xFFE3;

fn captured_files(dir: &std::path::Path) -> usize {
    fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

#[test]
fn capture_with_the_keyboard_twice_and_open_and_close_the_help() {
    if !enabled() {
        return;
    }
    let tmp = std::env::temp_dir().join(format!("snapforge-workflow-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let auto = tmp.join("auto");
    fs::create_dir_all(&auto).unwrap();
    let (_app, win) = launch(
        "workflow",
        r#"{"out_dir":"OUT"}"#,
        &[("SNAPFORGE_TEST_AUTOSAVE_DIR", auto.to_str().unwrap())],
    );
    let input = Input::new();
    input.click((
        (win.rect.x + win.rect.w as i32 - 40) as i16,
        (win.rect.y + win.rect.h as i32 - 40) as i16,
    ));
    let empty = screen();

    input.with_mods(&[CONTROL_L], "n");
    wait_until("the first capture", 20, || {
        (captured_files(&auto) == 1).then_some(())
    });
    sleep(Duration::from_millis(1500));
    let after_first = screen();
    keep("workflow-after-first-capture", &after_first);
    assert!(
        differing_pixels(&empty, &after_first) > 5000,
        "the empty state is replaced by the capture and its action bar"
    );

    // F1 opens the shortcut list over the window and Escape closes it again.
    input.tap("F1");
    sleep(Duration::from_millis(600));
    let help = screen();
    keep("workflow-help", &help);
    assert!(
        differing_pixels(&after_first, &help) > 5000,
        "help is shown"
    );
    input.tap("Escape");
    sleep(Duration::from_millis(600));
    let closed = screen();
    assert!(
        differing_pixels(&after_first, &closed) < differing_pixels(&after_first, &help) / 4,
        "Escape closes the help"
    );

    input.with_mods(&[CONTROL_L], "n");
    wait_until("the second capture", 20, || {
        (captured_files(&auto) == 2).then_some(())
    });
    sleep(Duration::from_millis(1500));
    keep("workflow-after-second-capture", &screen());
    let _ = fs::remove_dir_all(&tmp);
}
