# Testing

Everything runs in Docker. `make test-all` runs the full gate in order: lint, unit and integration
tests, live X11 tests, the window end-to-end suites, HiDPI, the release build and the
clean-machine check. `make coverage` writes a line-coverage report to `dist/coverage/`
(`summary.txt`, and `html/index.html`).

| Command | What it runs |
|---|---|
| `make lint` | `cargo fmt --check`, `cargo clippy -D warnings` |
| `make test` | unit and integration tests (no display needed; X11 tests skip themselves) |
| `make test-x11` | live capture and global-shortcut tests on Xvfb, plus settings checks through the real binary |
| `make test-guide` | the real binary on a virtual display with fake mouse and keyboard: guide window, composition preview, export, themes, pseudo-language, Tab focus, first-run capture workflow |
| `make test-editor` | the annotation editor, every tool, undo, save, reopen, export |
| `make test-hidpi SCALE=2` | capture sizes with a scaled interface |
| `make portable-check` | the release binary on a bare Ubuntu image with only the system libraries |
| `make package` / `make package-check` | builds the tarball, `.deb` and `.rpm` for x86_64 and aarch64; installs them in clean Ubuntu, Debian and Fedora containers (unprivileged tarball install, dry run, uninstall leaves nothing, deb remove/purge, rpm remove). aarch64 artifacts are only checked to be aarch64 files: there is no emulation here, so they are never executed |
| `make coverage` | all of the above except portable-check, instrumented |

## Where each item of the plan is tested

| Plan item | Tests |
|---|---|
| Project and item models | `guide-core` `project::tests` (ids never reused, inclusion, validation, plain vs guide kind), `guide::tests` (composition settings) |
| Annotation models, undo and redo | `annotation::tests`, `history::tests` (including random edit sequences undone and redone) |
| Project serialization, schema validation | `project::tests::json_round_trip...`, `validate_catches...`, `guide-storage/tests/golden.rs` (frozen version 1 guide and plain projects that must keep opening) |
| File path validation | `safe_and_unsafe_asset_paths`, `asset_path_resolution_rejects_traversal`, `a_symlink_that_leaves_the_project_is_never_followed`, `unsafe_image_paths_in_a_manifest_refuse_to_open` |
| Step reordering, duplication, deletion | `guide-core`, `guide-storage`, `project::tests`, and `guide_e2e` through the window |
| Text layout, image scaling, composition dimensions | `guide-render` `compose::tests`, `text`, `resample` |
| Export naming and rendering | `guide-export` tests (including plain projects), `export_e2e` |
| Click capture | `guide-capture` `session::tests` (click coordinates per mode, window under the click, debounce, limit, cancel), `guide-hotkeys` `x11_live` (raw XInput2 click, wheel ignored), `project::tests` (marker annotation), `click_e2e` |
| Profiles | `guide-storage` `settings::tests` (seeding, round trip, cleanup, add/rename/delete rules), `editor::props::tests` (options conversion), `profile_ui::tests` (name rules) |
| Capture orchestration | `guide-capture` `session::tests`, `workflow_e2e` (Ctrl+N twice through the window) |
| Saving and reopening screenshots and guides | `image_out::tests`, `edit_bundle::tests`, `project_store::tests`, `editor_e2e`, `guide_e2e` |
| Imported images | `importing_converts_to_png...`, `importing_a_missing_or_bad_file...` |
| Missing assets | `missing_and_unreadable_images_open_with_clear_issues`, `a_missing_screenshot_is_drawn_as_a_labelled_placeholder`, `check_cli` |
| Corrupt project files | `a_damaged_manifest_is_recovered_from_the_backup...`, `when_both_files_are_damaged...`, `a_manifest_with_the_wrong_shape...` |
| Unsafe or oversized files | `edit_bundle::tests` (`an_original_that_claims_a_huge_size...`, `an_oversized_annotations_file...`, `links_in_place_of_the_two_files...`), `project_store::tests` (`a_link_in_place_of_the_assets_folder...`, `a_symlink_that_leaves_the_project...`), `settings::tests` (`a_huge_settings_file...`) |
| Interrupted or failed writes | `saving_is_atomic...`, `leftover_temp_files_from_an_interrupted_save...`, `a_failed_replace_leaves_the_old_file...`, `a_temp_file_left_by_a_crash...`, `many_threads_saving_the_same_file...`, `without_hard_links_the_file_is_copied...`, `failed_save_reports_the_path...`, `a_failed_save_raises_a_banner...` |
| Window capture, region selection, cursor | `windows::tests`, `region::tests`, `cursor::tests`, `x11_live` |
| Unsupported capture capabilities | `backend::tests`, `guide-hotkeys` `wayland_reports_unsupported_per_binding` |
| HiDPI | `region::tests::hidpi...`, `hidpi_e2e` (interface scale 2) |
| UI workflows | `guide_e2e`, `composition_e2e`, `export_e2e`, `editor_e2e`, `a11y_e2e`, `workflow_e2e`, `click_e2e` (click capture: shortcut-started session saves one file per click and stops, clicks become guide steps with a marker while the stored image keeps its size, `snapforge clicks --count`), `gui_e2e` (`snapforge gui`: overlay, region, Enter copies and the clipboard outlives the program, Ctrl+S saves with `--path`, Esc cancels, delay, no-display and bad-argument exits) |
| Translations and accessibility | `i18n_guard`, `theme::tests` (contrast), `project::tests` in `guide-app` (screen-reader labels), `a11y_e2e` |

## Coverage

Last measured with `make coverage` (all suites together): **86% of lines**. The logic crates
(`core`, `render`, `export`, `storage`, `capture` without the X11 glue) are at 92-100%. Lowest
areas: `app.rs` (main window, about 42%: region overlay, global-shortcut handling and settings
panels are only exercised by hand), `main.rs` (start-up and `--self-test`).

## What is NOT tested, and why

* **Wayland.** Capture and global shortcuts are not implemented there (see
  `platform-support.md`); only the detection and the "unsupported" reporting are tested. There is no
  Wayland session in the Docker setup.
* **Multiple monitors.** The region and monitor arithmetic is unit tested with offsets, but the
  virtual X servers used here have one screen, so a real two-monitor setup is untested.
* **Real desktops.** All end-to-end tests run on Xvfb/Xvnc with Openbox. Behaviour on XFCE, GNOME,
  KDE and with a compositor was checked by hand on one machine (Manjaro, XFCE, X11) only.
* **Capture permission prompts** (portals) do not exist on X11; cancellation of a running capture is
  tested (`cancel_before_start...`, `cancel_during_countdown...`).
* **Screen readers.** The accessibility tree is tested; Orca itself was not.
* **Clipboard managers.** The copy path is tested without one (the background server owns the clipboard); behaviour with GNOME/KDE/XFCE clipboard managers is untested.
* **`snapforge gui` on several monitors.** Overlay placement on the monitor under the pointer is implemented but was only run on one virtual monitor.
