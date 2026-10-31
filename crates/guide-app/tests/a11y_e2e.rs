//! End-to-end checks of themes, language and keyboard navigation through the real binary.
//!
//! `scripts/test-guide.sh` provides Xvnc and Openbox. Skipped unless SNAPFORGE_E2E=1.
mod common;
use std::{thread::sleep, time::Duration};

use common::*;

#[test]
fn the_light_and_dark_themes_are_really_applied() {
    if !enabled() {
        return;
    }
    let (_dark, win) = launch("dark", r#"{"out_dir":"OUT","theme":"Dark"}"#, &[]);
    let dark = screen();
    keep("theme-dark", &dark);
    let dark_level = brightness(&dark, &win.rect);
    drop(_dark);
    let (_light, win) = launch("light", r#"{"out_dir":"OUT","theme":"Light"}"#, &[]);
    let light = screen();
    keep("theme-light", &light);
    let light_level = brightness(&light, &win.rect);
    assert!(dark_level < 90.0, "dark theme brightness {dark_level}");
    assert!(light_level > 150.0, "light theme brightness {light_level}");
}

#[test]
fn the_pseudo_language_changes_the_visible_text() {
    if !enabled() {
        return;
    }
    let (_en, _) = launch(
        "english",
        r#"{"out_dir":"OUT"}"#,
        &[("SNAPFORGE_LANG", "en")],
    );
    let english = screen();
    drop(_en);
    let (_ps, _) = launch(
        "pseudo",
        r#"{"out_dir":"OUT"}"#,
        &[("SNAPFORGE_LANG", "pseudo")],
    );
    let pseudo = screen();
    keep("language-pseudo", &pseudo);
    assert!(
        differing_pixels(&english, &pseudo) > 2000,
        "the pseudo-language should redraw the labels"
    );
}

#[test]
fn tab_moves_the_visible_focus_through_the_controls_and_back() {
    if !enabled() {
        return;
    }
    let (_app, win) = launch("tab", r#"{"out_dir":"OUT"}"#, &[]);
    let input = Input::new();
    // Click an empty spot in the lower part of the window to give it keyboard focus.
    input.click((
        (win.rect.x + win.rect.w as i32 - 40) as i16,
        (win.rect.y + win.rect.h as i32 - 40) as i16,
    ));
    sleep(Duration::from_millis(400));
    let mut shots = vec![screen()];
    for _ in 0..6 {
        input.key_raw(0xFF09, true);
        input.key_raw(0xFF09, false);
        sleep(Duration::from_millis(400));
        shots.push(screen());
    }
    keep("keyboard-tab-6", shots.last().unwrap());
    for i in 1..shots.len() {
        assert!(
            differing_pixels(&shots[i - 1], &shots[i]) > 20,
            "Tab press {i} did not visibly move the focus"
        );
    }
    // Shift+Tab walks back to what was shown after the fifth press.
    input.with_mods(&[SHIFT_L], "Tab");
    sleep(Duration::from_millis(400));
    let back = screen();
    assert!(
        differing_pixels(&back, &shots[5]) < differing_pixels(&back, &shots[6]),
        "Shift+Tab should return towards the previous control"
    );
}

#[test]
fn the_settings_page_opens_and_a_picture_of_it_is_kept_for_review() {
    if !enabled() {
        return;
    }
    let (_app, _win) = launch(
        "settings-page",
        r#"{}"#,
        &[("SNAPFORGE_TEST_OPEN_SETTINGS", "1")],
    );
    sleep(Duration::from_millis(2500));
    keep("settings-page", &screen());
}
