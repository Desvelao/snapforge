# Changelog

## Unreleased

- **Removed.** The "Interface size" setting and the Ctrl+Plus / Ctrl+Minus / Ctrl+0 zoom shortcuts. An old `ui_scale` value in `settings.json` is ignored.
- **Icon.** New application icon: a capture frame around a forge spark on an anvil.
- **Renamed.** The application has a new name; the display name comes from `brand.toml`. The binary is `snapforge`, the config folder is `~/.config/snapforge`, and test variables start with `SNAPFORGE_`. Settings from a previous config folder are not carried over; the `.desktop` file, icons and documentation folder carry the new id, so remove an older package when installing.
- **Restyle.** The annotation editor follows the new mockup: a top bar (Undo, Redo, Delete, Clear crop | Fit/100% and Preview export | Save edit, Export image as the one accent button, Close), a tool palette of 2-column tiles with painted icons and shortcut letters, a properties panel on the right (colour swatches, width, opacity, fill, Delete annotation) in place of the old toolbar rows, and a monospace status bar. The Settings page uses a 720-point column, a segmented theme control and roomier rows. Behaviour, shortcuts and file formats are unchanged; the opacity slider sets the colour's alpha.
- **Enhancement.** In the editor, the mouse wheel now moves the picture (Shift+wheel sideways) and Ctrl+wheel or a pinch zooms, as in the guide's preview; before, any scroll zoomed and sideways scrolling did nothing.
- **Enhancement.** With the Crop tool, the eight handles of the existing crop can be dragged to adjust it (one undo step each) instead of redrawing it, and a drag too small to use says so in the status line instead of being ignored.
- **Enhancement.** The editor no longer asks about unsaved changes when you undo back to what was saved; it compares the picture's annotations and crop with the saved ones instead of counting changes.
- **Enhancement.** Cancelling a combined export now stops drawing at once instead of painting placeholders for every remaining step; steps are also looked up in a map rather than searched for each image.
- **Enhancement.** Text in other scripts (CJK, Hebrew, Arabic, Devanagari...) in annotations, titles and exports now uses an installed system font where the embedded one has no glyph. Nothing was added to the program size; right-to-left order and joined letter shapes are still not handled.
- **Enhancement.** The editor remembers the last 500 changes for undo instead of growing without limit, and dragging a slider or colour back to where it started no longer leaves an empty undo step.
- **Enhancement.** Monitor capture identifies a monitor by its RandR name instead of its position in the list, so plugging monitors in or out no longer changes which one is meant; a server without RandR offers the whole screen as one monitor.
- **Enhancement.** The language picker shows each language by its own name (a `"_language"` entry in its translation file) instead of the code. No translation ships yet, so only English is listed today.
- **Tests.** The editor's tool-settings round trip is now tested for every annotation kind (line, arrow, pen, ellipse, both blur modes).
- **Fix.** Emptying a guide or project name no longer leaves the window title blank: "Untitled guide" / "Untitled project" is shown (and as the field's hint). The exported image already skipped an empty title and the file name falls back to "guide".
- **Fix.** The inspector's text fields, number boxes, slider and drop-downs now have names for screen readers, and a test checks every input on every inspector tab.
- **Fix.** The toolbar Export button and Ctrl+E no longer start an empty export when no step is included; they say "No steps are included in the export." like the Export now button does.
- **Fix.** The export "File names" examples show the extension of the chosen image format (`.jpg`, `.webp`) instead of always `.png`.
- **Fix.** Closing a project while an export is running now stops the export first (waiting up to 10 seconds) instead of deleting unused screenshots from under it.
- **Fix.** In the project window, Ctrl+S, Ctrl+E and Ctrl+D now work while you are typing in a text field (title, description, notes); plain keys such as Delete and the arrows still stay with the field.
- **Tests.** Tests for chosen file names with odd extensions (`foo.tar`, `.hidden`) and for leftover temp files from a crashed save; `docs/testing.md` lists the new safety tests.
- **Fix.** A link standing in place of a project's `assets/` or `exports/` folder is refused for writing and for deleting unused screenshots, and asset paths now resolve to the real checked path.
- **Fix.** A settings file with one value this version cannot read (for example an option from a newer version) now resets only that value instead of all settings; a newer version's file is copied to `settings.json.v<N>` before this version can overwrite it; damaged files no longer replace an earlier `.bad` file, and a huge file is set aside unread.
- **Fix.** Saving screenshots no longer fails on drives that cannot make hard links (FAT/exFAT sticks, some network and FUSE mounts); the file is copied instead, still without overwriting anything.
- **Chore.** All file writes of the storage crate (projects, edit folders, settings, images) go through one atomic-write helper that uses a unique temp name per save and flushes the folder after the rename.
- **Fix.** Opening or creating an edit folder (`.guideedit`) now has the same safety limits as a project: huge images and huge annotation files are refused, links in place of its two files are refused, an existing original is never replaced, and a failed create removes what it made.
- **Fix.** Recovering a damaged `project.json` from its backup is now one atomic replace (there is never a moment without a manifest), and a damaged file kept by an earlier recovery is no longer overwritten (`project.json.damaged.1`, ...).
- **Tests.** Headless tests for the editor: committing typed text, saving a step edit versus an edit folder, closing with unsaved changes, and the Escape key.
- **Chore.** Removed a leftover no-op in the editor preview painting.
- **Fix.** The editor's "100% (1)" zoom button is now translatable like "Fit (0)".
- **Fix.** In the editor, Redo and Delete now finish a text you are still typing first, so the text box cannot be left pointing at an annotation that is gone.
- **Fix.** In the editor, Delete, Undo/Redo and the tool keys no longer change the picture while "Preview export" is showing.
- **Tests.** More export and redaction tests: step numbers in separate images with text, per-step description override, number-only names for 100+ steps, and obscure regions partly outside the image, with extreme strengths, and under a crop.
- **Fix.** The grey "Screenshot missing" placeholder in the combined export and the preview is now in the interface language.
- **Chore.** `Action::label` is documented as the untranslated diagnostic name; the window already translates action names.
- **Fix.** `Ctrl++R` in a shortcut now says it has an empty part (two `+` in a row) instead of naming an unknown modifier with an empty name.
- **Fix.** A damaged or hand-edited project or edit file with an id near the 64-bit limit is refused with a clear message instead of wrapping around and handing out an id that is already in use.
- **Docs.** The project format notes that colours are never corrected (a transparent or background-coloured text colour gives invisible text).
- **Fix.** Opening a project now also drops saved layouts of steps that no longer exist and clamps per-step image widths, and tells you in the notice (as it already did for the guide-wide settings).
- **Fix.** The title of a duplicated step (`... (copy)`) is now translatable.
- **Chore.** The shortcut debounce and modifier masking are now plain functions with unit tests, and the click position limitation (read just after the release) is documented.
- **Chore.** Listing windows for window capture is faster: atoms are looked up once and the per-window X requests are sent together. Titles of old clients that use Latin-1 `WM_NAME` now keep their accents.
- **Fix.** A click session no longer ends when one capture fails (for example a window closing under the click); that click is skipped and the session stops only after three failures in a row.
- **Fix.** Global shortcuts now also find letter keys in a second keyboard layout group (e.g. Latin as the second layout next to Cyrillic) and are grabbed again when the keyboard layout changes.
- **Fix.** A full-screen capture after the screen size changed (xrandr, a monitor plugged in) used the size from when the program started; it now asks the X server each time.
- **Docs.** `settings.md` no longer names a development phase and says its JSON example is shortened.
- **Fix.** The `.deb`/`.rpm` dependencies, `install.sh` check and install docs now list the same X11 libraries the program loads (`libXrender`, `libX11-xcb`, `libxcb` were missing), matching `portable-check`.
- **Chore.** The development image no longer installs `cargo-deny`, which nothing used (shorter image build).
- **Fix.** The clipboard hand-over folder is now checked when it already exists: it must be a real folder (not a link) owned by you and closed to others, and it is named by user id instead of `$USER`.
- **Fix.** `--help` described the `--check` exit codes backwards; it now says 1 = cannot be opened, 2 = problems found, and a test keeps it honest.
- **New look.** The interface follows the new mockups: a dark palette (panel `#14161A`, cards `#1B1E24`,
  accent `#FF8A3D`), 8 px widget and 12 px card corners, and Inter and JetBrains Mono (shortcuts and paths),
  both embedded under the SIL Open Font License. The home screen has the capture mode as a 2x2 grid, the
  options in one card, Capture as the only accent button, tabs for Preview, Projects and Saved edits, and
  the save bar pinned to the bottom. The project window has a top toolbar with Export as the accent
  button, the steps on the left, and the properties as tabs (Guide, Step, Layout, Numbering) with the
  "Show in export" switches in a two-column grid; the capture options of an open project are in the
  "Capture and files" window. The `.guideproj` format, shortcuts and settings are unchanged. The light
  theme keeps its colours. The border of buttons and fields stays lighter than the mockup's card border so
  that it keeps the 3:1 contrast the accessibility tests require.
- **Profiles.** Named sets of tool settings: `click_capture_options` (mark clicks, max shots) and
  `editor_options` (colour, widths, text size, bold, fill, blur mode and strength, marker size).
  Choose, create, rename and delete profiles in the "Click capture" row and in the editor's tool
  bar. A new profile starts as a copy of the current one, and the profile used last is selected again
  on start. Click markers use the profile's colour. Stored as `profiles` and `active_profile` in
  `settings.json`; older files get a "Default" profile made from their `click_marker` and `click_max`.
- **Click capture.** A session that takes a screenshot of every left click. Start and stop it with the
  new global shortcut Ctrl+Shift+C (setting `toggle_click_capture`) or the "Click capture" button under
  "Capture and files"; Esc also stops it while the window has focus. The capture mode decides what each
  click grabs (full screen, single monitor, or the window under the click; Region mode grabs the full
  screen). With a project open every click becomes a new step, otherwise the file goes to the output
  folder. "Mark clicks" (setting `click_marker`, off by default) adds a numbered marker annotation at the
  click; the stored screenshot is never changed. "Max shots" (`click_max`, 0 = no limit) ends the session
  by itself. `snapforge clicks [--count N] [--path DIR]` does the same without a window. X11 only; it
  needs the XInput2 extension.
- **Projects.** Guides now live under a broader feature, Projects: a project is a set of screenshots to
  annotate and export, and a guide is one kind of project. A **plain project** keeps a title, description
  and caption for each screenshot and exports separate images; a **guide** adds the introduction, final
  notes, numbering, layout settings and the combined image. The main window has a Projects section with
  a Guide / Plain project choice, "Add to project" and "Close project"; `snapforge --check` reports the kind.
- **Breaking:** the `project.json` layout was rewritten (`name`, `kind`, `items`; guide settings inside
  `kind`). The schema version stays 1, so folders made by earlier builds are refused with a clear message
  instead of being upgraded. The `recent_guides` setting is now `recent_projects` (the old list is not carried over).
- A gear button in the header opens a Settings page with Shortcuts and Appearance, which left the capture controls.
- `snapforge gui`: a list of shortcuts (copy to clipboard, save the screenshot in a file, cancel,
  undo, tools...) stays in the corner of the selection screen and of the editor. It hides while
  the mouse is dragging or drawing and with F1, and is left out when the image is too small to spare the room.
- `snapforge gui`: the shortcut list is centered on the selection screen. No small window flashes
  any more when it starts (it opens at the monitor's size and position) or when Esc cancels.
- `snapforge gui`: Ctrl+S / "Save as file" opens the system save dialog (desktop portal) to pick the
  folder and name; cancelling keeps the editor open. Without a portal it saves a timestamped file
  into the screenshots folder as before.
- "Browse..." buttons next to the folder and file fields (screenshots folder, open guide, open
  saved edit, import image, export folder) open the system dialog.
- `snapforge gui`: one-shot capture without the main window (frozen screen, region drag, editor,
  Enter copies, Ctrl+S saves, Esc cancels; `--delay`, `--path`). Meant for a keyboard shortcut.
- Copy now survives closing the application: a small background process keeps serving the clipboard
  image until something else is copied (X11 clipboards otherwise vanish with their owner).
- Menu entry gains a "Capture a region" action.

## 0.1.0 - 2026-10-29

First release.

**Works**
- Capture on X11: full screen, one monitor, region, window, optional cursor, delay, cancel.
- Global shortcuts (X11) and settings saved between runs.
- Annotation editor with all tools, undo/redo, crop, blur, saved edits that can be reopened.
- Guides: steps, reorder/duplicate/exclude/remove, import images, composition preview,
  separate-image and combined-image export, autosave with backup recovery, `--check`.
- Light and dark themes, interface size, keyboard navigation with a visible focus ring,
  shortcut help (F1), recent guides and edits, error dialogs.
- Packaging: portable archive with a no-root installer, `.deb` and `.rpm` for x86_64 and aarch64.

**Known limits**
- Wayland: capture and global shortcuts are not implemented.
- English only (a translation framework exists; no translations are shipped).
- Not verified with a real screen reader; annotations need a pointer.
- The `.deb` (Ubuntu 22.04, Debian 12) and `.rpm` (Fedora) x86_64 packages were install-tested in containers. Tested on Debian 12 and Ubuntu 22.04 containers and by hand on Manjaro/XFCE/X11; other
  distributions and desktops are untested. The aarch64 packages are built but have not been run
  on arm64 hardware.
- No AppImage is provided.
