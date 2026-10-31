# Project format

A **project** is a set of screenshots that can be annotated and exported. A project has a *kind*:

- **Plain project:** an ordered list of screenshots. Each one has a title, a description and a caption, can be annotated,
  cropped and left out of exports, and is exported as a separate image.
- **Guide:** a project that is also a numbered step-by-step guide: it adds an introduction, final notes, a composition
  (layout, numbering, colours) and per-step overrides, and can be exported as one combined image (see `docs/composition.md`).

Status: the project model (`guide-core`), project storage (`guide-storage`) and the project window
(three panels) are implemented and tested. See "Using projects" and "Known limits".

## Folder
```
my-project.guideproj/
  project.json        the project; replaced atomically on every save
  project.json.bak    the previous save
  assets/             original screenshots, written once, never modified
  exports/            exported images
```
Only relative names are used, so the folder can be moved or copied. Original images are stored as files,
never as base64 inside the JSON.

## project.json
A guide:
```json
{
  "schema_version": 1,
  "project_id": "p-0f3a9c1d5b7e2468a1c3e5f7092b4d6f",
  "name": "How to change a setting",
  "kind": {
    "type": "guide",
    "introduction": "",
    "final_notes": "",
    "composition": {
      "layout": "Vertical", "background": [255, 255, 255, 255], "text_color": [32, 32, 32, 255],
      "margin": 32, "spacing": 24, "max_width": 1200, "font_size": 20.0,
      "image_scaling": "ShrinkToFit", "image_align": "Left", "description_placement": "AboveImage",
      "show_numbers": true, "show_titles": true, "show_descriptions": true,
      "show_guide_title": true, "show_introduction": true, "show_final_notes": true
    },
    "step_layouts": { "1": { "max_width": 600, "show_description": false } }
  },
  "items": [
    { "id": 1, "title": "Open settings", "description": "Click the gear icon.", "caption": "The gear icon",
      "image": "assets/shot-2026-10-28_18-30-00.png",
      "annotations": { "version": 1, "annotations": [], "crop": null, "next_id": 1 },
      "included": true }
  ],
  "next_item_id": 2
}
```
A plain project has `"kind": { "type": "plain" }` and no guide settings; everything else is the same.
- `project_id` is fixed when the project is created and never changes.
- `kind.type` is `"plain"` or `"guide"`. Guide-only data (introduction, final notes, composition, `step_layouts`) lives
  inside `kind`, so a plain project cannot carry it.
- Item `id`s are stable and never reused, even after an item is deleted. Reordering, duplicating and
  deleting other items never changes an item's id or its `image`. `step_layouts` is keyed by item id.
- `annotations` is the same document the annotation editor saves (see `docs/annotations.md`).
- `included: false` keeps an item in the project but leaves it out of exports.
- Optional fields have defaults, so a hand-written item only needs `id`, `title` and `image`, and a guide
  without a `composition` block uses the defaults. The meaning of each composition setting is in `docs/composition.md`.
- Folders written by earlier builds (a top-level `title` and `steps`, no `kind`) are not readable any more: they are refused
  with a message saying they use an older layout. The format version number stayed at 1.

## Safety rules
- A step image must be `assets/<name>` where `<name>` uses only letters, digits, `.`, `_`, `-` and does not
  start with a dot. Absolute paths, `..`, nested folders, backslashes and other spellings are refused.
  A manifest that contains one is **not opened** (it is not something this app writes), and the app refuses
  to save such a project.
- Even a safe-looking name is checked against the real file: a symlink in `assets/` that points outside the
  project is reported as a problem and is never read or deleted through.
- Original screenshots are never overwritten: new ones get a new name (`shot-<timestamp>[-n].png`).
- Importing PNG, JPEG or WebP converts to a PNG asset and leaves the source file untouched. Images over
  120 million pixels are refused before they are decoded.

## Saving and recovery
1. If `project.json` is readable it is copied to `project.json.bak`.
2. The new manifest is written to a temporary file in the same folder, flushed, and renamed over `project.json`.

A crash at any point leaves either the old or the new manifest, never half of one. If `project.json` is
nevertheless unreadable (disk fault, manual edit), opening restores the previous save from `project.json.bak`,
keeps the bad file as `project.json.damaged` (`.damaged.1`, `.damaged.2`... if an earlier one exists), and tells the user the last changes may be missing. A
damaged file is never used to overwrite a good backup. If both are unreadable the error names the file and
nothing is deleted. A file saved by a *newer* version is refused with a message and is never "recovered" over.

## Missing or unreadable images
A project still opens when a screenshot is missing, is not a readable image, is too large, escapes the project,
or when an item's annotations do not fit its image. Each case is returned as an `Issue` naming the item and
the file, for the window to show, and the other items remain usable.

## Deleting items and cleaning up images
Deleting an item only edits the manifest. `prune_unreferenced_assets` then removes images that no item refers
to. A duplicated item shares its original's image, so the image stays until every item using it is gone.
Only files the app created (`shot-*.png`) are ever removed; other files in `assets/` and symlinks are left alone.

## Schema versions and migrations
`schema_version` is the format of `project.json`. When a change would break older readers, the version is
bumped and a migration (a function that rewrites the parsed JSON from version N to N+1) is added to the list in
`crates/guide-storage/src/project_store.rs`. Opening runs every migration needed, in order, and tells the
user what was upgraded; the pre-upgrade file becomes `project.json.bak` on the next save. Adding an optional
field with a default does not need a new version. There are no migrations yet: the project layout was rewritten while the version stayed at 1, so folders from
earlier builds are refused rather than upgraded. The mechanism is tested with a stand-in chain.

## Compatibility rules (for people changing the format)
These decide when the version number changes. They are what keeps old files working.
- **Adding an optional field with a default** does not change `schema_version`. Older versions that open such a file keep
  the fields they do not know when they save: unknown fields are preserved at the project, guide-settings, item, composition and
  annotation-document level, so a newer build's data is not lost by an older one.
- **Renaming or removing a field, changing a type or what a value means, or adding a required field** changes `schema_version`
  and needs a migration.
- **Adding a new annotation type changes `schema_version` too.** An older version cannot keep an annotation it cannot read, so it
  must refuse the file with the clear "saved by a newer version" message instead of failing to parse it.
- `crates/guide-storage/tests/fixtures/project-guide.guideproj` (a guide with every annotation type) and
  `project-plain.guideproj` (a plain project) are hand-written format 1 projects. They are committed once and **must never be regenerated**: `make test` opens it, checks its content, and checks a save and
  reopen changes nothing. If that test fails, the change broke existing users' files; fix the code or add a migration, not the fixture.

## Keeping things apart
- `assets/` holds only original screenshots, each written once and never modified.
- Editable annotations are in `project.json`, never drawn into the originals.
- Derived pictures (step thumbnails, the composition preview) are drawn in memory and not stored, so there is nothing
  stale to clean up and nothing in the folder that is not either an original or the manifest.
- Exported images go to the output folder (by default `exports/`), never into `assets/`.
- Screenshot files are named `shot-<timestamp>.png` and are never renamed, because several items can share one (a duplicated
  item) and references must stay valid however steps are reordered. The order and numbering come from the manifest, not file names.
- Copying the folder copies the project, including its `project_id`; two copies are the same project to the app.

## Checking a project from the command line
`snapforge --check <folder>` opens the project without a window and prints what it found: the kind and name, item counts, anything
that was repaired or upgraded, every problem with a screenshot, and, for a guide, the size the combined image would have.
Exit codes: **0** healthy, **1** cannot be opened, **2** opens but has problems (including "recovered from backup").

## What the errors mean
| Message says | Cause | What to do |
|---|---|---|
| `... is not a project` | no `project.json` (and no usable backup) in that folder | choose the folder that contains `project.json` |
| `the project file ... is damaged: ... line N` | `project.json` is not valid JSON or has the wrong shape (missing or mistyped field) | fix the line named, or restore `project.json.bak`; if the backup is fine, the app restores it for you and keeps the bad file as `project.json.damaged` |
| `saved by a newer version` | the file's format version is higher than this build knows | use a newer version; the file is not touched |
| `cannot read <path>: Permission denied` (or `Is a directory`) | the file cannot be read | fix permissions or the path |
| `too large (N MB; the limit is 64 MB)` | `project.json` is not something this app wrote | check the file |
| `refers to an unsafe image path` | an item points outside `assets/` (`..`, absolute path, subfolder) | the project is refused to protect your files; correct the path |
| `the screenshot ... is missing` / `not a readable image` | the file in `assets/` was deleted, moved or damaged | restore it; the rest of the project still opens and works |
| `composition.margin 99999 is outside 0-2000; using 2000` | a setting was out of range (hand-edited file) | the value was corrected; save to keep it |
| (no message) text is invisible in the export | `text_color` is transparent or the same as `background`; colours are never corrected, any RGBA value is accepted | set another `text_color` |
| `unknown variant ... newer version` | an annotation type from a newer version | use a newer version |
| `saved with an older layout of project.json` | a folder from a build before projects existed (top-level `title` and `steps`) | open it with that build; this version cannot read it |

## Limits and known gaps
- `project.json` is limited to 64 MB and a single screenshot to 120 million pixels.
- Only one instance of the app should have a project open at a time. There is no lock file: two windows editing the same folder will overwrite
  each other's saves (the `.bak` keeps the one before).
- Only one backup (the previous save) is kept.

## Using projects
Open the **Projects** section of the main window.

- **New project:** choose **Guide** or **Plain project**, type a title and press Create. The project is a folder named after
  the title (`<output folder>/<title>.guideproj`; `-2`, `-3`... if the name is taken).
- **Open project:** pick one from the recent list, or paste the folder path and press Open. A damaged `project.json` is
  recovered from the backup (you are told), and items with missing or unreadable screenshots are listed under "Problems"
  and marked `(!) image problem` in the list; the rest of the project stays usable.
- **Add screenshots:** take a capture and press **Add to project**; turn on **Add captures to the project** to add every
  capture (button, Ctrl+N or a global shortcut); or paste an image path under the project properties and press Import.
  A new item goes right after the selected one, or at the end.
- **List (left):** thumbnail, number and title. Drag an item to reorder it (a blue line shows where it will
  land; the list scrolls when you drag to its top or bottom edge). Excluded items are dimmed and marked.
- **Selected item (right):** edit its title, description and caption, tick or untick **Include in export**, and use
  Move up / Move down, **Duplicate**, **Remove** (asks first) and **Edit screenshot**, which opens the annotation
  editor on that item. In the editor, Ctrl+S or **Done** (Ctrl+W) keeps the annotations in the item;
  **Cancel** discards what was done since the last save.
- **Guide only:** introduction and final notes, per-step overrides, the layout, numbering and colour settings, the
  preview of the combined image and the **One combined image** export. A plain project shows the selected
  screenshot as it will be exported instead.
- **Keys** (when no text box has focus): Up/Down select, Alt+Up/Alt+Down move the item, Ctrl+D duplicates,
  Delete removes (Enter confirms, Esc cancels), Enter edits the screenshot, Ctrl+S saves.
- **Saving:** changes are saved automatically about a second and a half after you stop, and when you close the
  project or the window. Removing an item does not delete its screenshot straight away; unused screenshots are
  cleaned up when you leave the project, so an accidental removal cannot lose an image.

## Known limits
- Captions are stored and edited for both kinds but are not drawn into the combined image or the separate images yet.
- There is no undo for item operations (removing, moving, duplicating); removal asks for confirmation instead.
  Annotation edits inside a step have full undo in the editor.
- Item previews are drawn from the original and its annotations on a background thread. For very long guides
  only the visible rows (plus the selected step) are drawn, and at most 64 previews are kept in memory.
- Dragging works in the visible part of the list; use Alt+Up/Down for long moves.
- Only X11 has been exercised, by fake mouse and keyboard on a virtual display at UI scales 1, 1.5 and 2, not
  yet by a person on a real desktop.

Exporting is described in `docs/export.md`.
