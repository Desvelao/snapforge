//! End-to-end check of click capture through the real binary: the shortcut starts a session,
//! every left click takes a screenshot, the shortcut stops it again.
//!
//! `scripts/test-guide.sh` provides Xvnc and Openbox. Skipped unless SNAPFORGE_E2E=1.
mod common;
use std::{fs, path::Path, thread::sleep, time::Duration};

use common::*;
use guide_core::{Kind, Project};
use guide_storage::ProjectStore;

const CONTROL_L: u32 = 0xFFE3;

fn pngs(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
                .count()
        })
        .unwrap_or(0)
}

/// The launch helper keeps its settings and default output folder here.
fn launch_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("snapforge-a11y-{tag}-{}", std::process::id()))
}

fn toggle(input: &Input) {
    input.with_mods(&[CONTROL_L, SHIFT_L], "c");
}

#[test]
fn every_click_in_a_session_is_saved_until_the_shortcut_stops_it() {
    if !enabled() {
        return;
    }
    let out = launch_dir("click-files");
    let (_app, _win) = launch("click-files", r#"{"out_dir":"OUT"}"#, &[]);
    let input = Input::new();

    toggle(&input);
    sleep(Duration::from_millis(3000)); // window minimizes, stale clicks are dropped
    input.click((60, 60));
    wait_until("the first click shot", 40, || {
        (pngs(&out) == 1).then_some(())
    });
    input.click((400, 300));
    toggle(&input);
    sleep(Duration::from_millis(3000));
    input.click((200, 200)); // after the session: no more shots

    assert_eq!(pngs(&out), 2, "two clicks in the session, none after it");
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn clicks_become_guide_steps_with_an_optional_marker_and_the_original_stays_clean() {
    if !enabled() {
        return;
    }
    let tmp = std::env::temp_dir().join(format!("snapforge-click-guide-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let proj_dir = tmp.join(format!("g.{}", guide_core::PROJECT_EXTENSION));
    let guide = Project::new_guide("Click guide");
    let store = ProjectStore::create(&proj_dir, &guide).unwrap();
    store.save(&guide).unwrap();

    let (_app, _win) = launch(
        "click-guide",
        r#"{"out_dir":"OUT","click_marker":true,"click_max":2}"#,
        &[("SNAPFORGE_TEST_OPEN_GUIDE", proj_dir.to_str().unwrap())],
    );
    let input = Input::new();
    toggle(&input);
    sleep(Duration::from_millis(3000));
    input.click((120, 90));
    input.click((500, 350)); // the limit of 2 ends the session by itself

    let saved = wait_until("two steps on disk", 40, || {
        let p = ProjectStore::open(&proj_dir).ok()?.project;
        (p.items.len() == 2).then_some(p)
    });
    for step in &saved.items {
        let marker = step
            .annotations
            .annotations
            .iter()
            .find_map(|a| match a.kind {
                Kind::Marker { center, .. } => Some(center),
                _ => None,
            });
        assert!(
            marker.is_some(),
            "each click step carries a marker annotation"
        );
    }
    let opened = ProjectStore::open(&proj_dir).unwrap();
    let stored = opened.store.load_asset(&saved.items[0].image).unwrap();
    let desktop = screen();
    assert_eq!(
        (stored.width, stored.height),
        (desktop.width, desktop.height),
        "the stored screenshot is the plain full-screen capture; the marker lives in the annotations"
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn the_clicks_command_saves_one_file_per_click_and_exits_after_the_count() {
    if !enabled() {
        return;
    }
    let out = std::env::temp_dir().join(format!("snapforge-clicks-cli-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_snapforge"))
        .args(["clicks", "--count", "2", "--path"])
        .arg(&out)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("start snapforge clicks");
    sleep(Duration::from_millis(3000)); // connects to the X server and starts listening
    let input = Input::new();
    input.click((80, 80));
    input.click((300, 250));
    let status = wait_until("the command to exit", 40, || {
        child.try_wait().ok().flatten()
    });
    assert!(status.success());
    assert_eq!(pngs(&out), 2);
    let _ = fs::remove_dir_all(&out);
}
