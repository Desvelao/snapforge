# Changelog

## Unreleased

## 0.1.0-alpha2 - 2026-11-05

- Editor keyboard access: the arrow keys move the selected annotation (Shift: 10 px; repeated nudges are one undo step), Tab / Shift+Tab cycle the selection while the canvas has focus, and screen readers hear the annotation count and the selected one on the canvas.

- The Projects tab now preselects **Plain** (formerly "Plain project") when creating a new project (it was Guide).

- New project type **Diff**: a previous and a post screenshot composed into one image, side by side or one above
  the other, with an optional header (title and description) and a caption under each screenshot. Create it from
  the Projects tab; export it as one combined image. `snapforge --check` validates it (two screenshots, image size).

- New setting "Show the window again after a capture": turn it off to keep the window minimized while
  captures are added to an open project.

- Repeat the last capture: the **Repeat last capture** button or `Ctrl+Shift+A` takes the same full screen,
  monitor, region or window again without the selection step. A window is found again by its title if its
  id is gone. If the window, monitor or region no longer exists the status says so. It is remembered only
  until the program closes.

- Desktop notifications after taking a screenshot (can be switched off separately), copying a screenshot, saving an image or an edit, saving a project (not on
  autosave) and finishing an export. The command is configurable under Settings > Notifications, which also
  has a button to send a test notification. Default: `notify-send` (libnotify), optional.

## 0.1.0-alpha1 - 2026-10-31

First (alpha) release.

**Capture (X11)**
- Full screen, one monitor (identified by its RandR name), region, window, optional cursor, delay, cancel.
- Global shortcuts and settings saved between runs.
- Click capture: a session that takes a screenshot of every left click (Ctrl+Shift+C or the "Click capture" button),
  optionally with a numbered marker at the click; `snapforge clicks [--count N] [--path DIR]` does it without a window.
- `snapforge gui`: one-shot capture without the main window (frozen screen, region drag, editor, Enter copies,
  Ctrl+S saves, Esc cancels; `--delay`, `--path`), meant for a keyboard shortcut.
- Copied images stay available after the application closes.

**Annotation editor**
- All tools, undo/redo (last 500 changes), crop with adjustable handles, blur/obscure, text in any script that an
  installed font covers, preview of the export, saved edits that can be reopened. The original screenshot is never modified.
- Profiles: named sets of tool settings and click-capture options.

**Projects**
- A project is a set of screenshots to annotate and export. A plain project exports separate images; a guide adds
  an introduction, final notes, step numbering, layout settings and a combined long image.
- Steps: reorder, duplicate, exclude, remove, import images; composition preview; autosave with backup recovery;
  `snapforge --check` validates a project.

**Interface**
- Dark and light themes, keyboard navigation with a visible focus ring, screen-reader labels, shortcut help (F1),
  recent projects and edits, system file dialogs, error dialogs.
- Translatable interface (English only for now).

**Install**
- One-line installer (`curl -fsSL .../get.sh | sh`) that verifies the checksum and installs for your user; portable
  archive with a no-root `install.sh`; `.deb` and `.rpm` for x86_64 and aarch64.

**Known limits**
- Wayland: capture and global shortcuts are not implemented.
- English only (no translations are shipped yet).
- Not verified with a real screen reader; annotations need a pointer.
- The x86_64 `.deb` (Ubuntu 22.04, Debian 12) and `.rpm` (Fedora) packages were install-tested in containers, and the
  program by hand on Manjaro/XFCE/X11; other distributions and desktops are untested. The aarch64 packages are built
  but have not been run on arm64 hardware.
- No AppImage is provided.
