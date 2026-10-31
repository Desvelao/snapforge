# AGENTS.md

## Overview
This is a Rust desktop app for Linux. It captures screenshots, annotates them, and assembles them into numbered step-by-step guides. Guides export as separate images or one long image.

- Capture and global shortcuts work on **X11 only**. Wayland is not implemented.
- `plan.md` is stale: it describes a Python/PySide6 design, but the code is Rust. Don't trust it.
- MIT licensed. Changes go in `CHANGELOG.md` under *Unreleased*.

## Stack
- Rust 1.88.0 (pinned in `rust-toolchain.toml`), edition 2021, 7-crate workspace.
- GUI: `eframe`/`egui` 0.31. Other deps: `image` (PNG only), `arboard`, `x11rb`, `serde`/`serde_json`, `thiserror`, `tracing`.
- Release profile is size-optimized (`opt-level = "z"`, fat LTO, `panic = "abort"`, stripped). The binary has a 15 MB budget.

## Layout
`crates/`:
- `guide-core`: pure models (annotation, guide, history/undo-redo, mode, shortcut).
- `guide-capture`: capture backend, session, region, cursor, windows, `x11.rs`.
- `guide-hotkeys`: global shortcuts (keysym, x11).
- `guide-render`: compose, text, shapes, obscure (blur), resample.
- `guide-export`: separate and combined image export.
- `guide-storage`: settings, `.guideproj` project folder (PNGs + JSON manifest, atomic saves, backup recovery), `edit_bundle`, `image_out`.
- `guide-app`: the `snapforge` binary. `src/` has `main.rs`, `app.rs`, `gui.rs`, `check.rs`, `clipboard.rs`, `i18n.rs`, `theme.rs`, `editor/`, `guide/`. `tests/` holds the e2e suites. `locales/template.json` is the translation template.

Elsewhere: `docs/` (install, platform support, settings, project format, testing, ...), `docker/`, `scripts/`, `packaging/`, `dist/` (build output, don't edit).

## Commands
Builds and tests are meant to run in Docker via `make`; no host Rust toolchain is needed.
- `make test`: `cargo nextest run --workspace`
- `make lint`: `cargo fmt --check` + `cargo clippy --workspace --all-targets -- -D warnings`
- `make run`: run the app against the host X display
- `make dist`: size-checked release binary into `dist/`
- `make test-x11`, `make test-guide`, `make test-editor`, `make test-hidpi SCALE=2`: Xvfb-based suites
- `make coverage`, `make package`, `make package-check`, `make portable-check`, `make icons`
- `make test-all`: full release gate

Without Docker: `cargo test --workspace`, `cargo fmt`, `cargo clippy`. E2E tests need X11/Xvfb.

CLI: `snapforge` (main window), `snapforge gui [--delay MS] [--path DIR]` (one-shot capture), `snapforge --check` (validate a guide), `snapforge --self-test`.

## Conventions
- Keep logic in the pure crates (core, render, export, storage); keep X11/UI glue thin.
- Clippy must pass with `-D warnings`; run `make lint` before finishing.
- Test names are full sentences (e.g. `a_failed_replace_leaves_the_old_file...`). `docs/testing.md` maps plan items to tests.
- All user-facing strings go through `i18n`; `i18n_guard` enforces this.
- Accessibility (screen-reader labels, focus ring, contrast) is tested; keep it working.
- The original screenshot is never modified; edits are stored separately.
- Project asset paths are validated against traversal and symlinks.
- Storage schema version 1 is frozen by the golden test `crates/guide-storage/tests/golden.rs`.
