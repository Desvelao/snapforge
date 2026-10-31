# Annotations and edit files

Status: the model, undo/redo, renderer, file format and the editor window are implemented and tested.
See "Known limits" for what the editor does not do yet.

## Coordinate system
Every coordinate is in **pixels of the original screenshot**: origin top-left, y down. Zoom, pan,
window size and HiDPI scale only change how the editor *draws* those numbers, never the numbers,
so annotations stay aligned on any display. A crop does not move annotations; it only selects
which part of the original is exported.

## Files
```
shot.guideedit/
  original.png        the screenshot, written once, never rewritten
  annotations.json    editable annotations, replaced atomically on every save
```
Only relative names are used, so the folder can be moved or copied. `EditBundle::create` refuses to
touch a folder that already has an `original.png`, and no API rewrites it.

## annotations.json
```json
{
  "version": 1,
  "annotations": [
    { "id": 1,
      "kind": { "type": "Rectangle", "rect": {"x":10.0,"y":10.0,"w":200.0,"h":80.0} },
      "style": {"color":[230,40,40,255], "width":4.0, "fill":null} },
    { "id": 2,
      "kind": { "type": "Obscure", "rect": {"x":50.0,"y":120.0,"w":300.0,"h":40.0},
                "mode": "Pixelate", "strength": 12 },
      "style": {"color":[230,40,40,255], "width":4.0, "fill":null} }
  ],
  "crop": {"x":0,"y":0,"w":400,"h":300},
  "next_id": 3
}
```
- `annotations` are back to front; later entries draw on top.
- `kind.type` is one of `Pen`, `Highlighter`, `Line`, `Arrow`, `Rectangle`, `Ellipse`, `Text`, `Marker`, `Obscure`.
- `id` is stable within the file and never reused, even after delete or undo.
- Loading validates: format version, finite coordinates, unique ids, and that the crop fits the image.
  A damaged or invalid file is reported with its path and left untouched.
- A file written by a newer version (higher `version`) is refused with a message instead of being misread.

## Rendering
`guide_render::render` draws the annotations onto a copy of the original, then applies the crop.
- **Blur and pixelate rewrite the real pixels** (pixelate: block average; blur: three box-blur passes),
  including whatever was drawn below them, so redacted content cannot be recovered from the export.
  Annotations added after (above) an Obscure stay sharp.
- Highlighter multiplies, so dark text under it stays readable.
- Text uses the embedded Ubuntu Light (licence in `crates/guide-render/assets/UFL.txt`). It has no bold
  face, so "bold" thickens the strokes with a second pass.
- Images over 120 million pixels are refused with a message instead of risking running out of memory.

## Undo and redo
All edits go through `Editor`, which records a command per change. Moves and resizes are a single
`Replace`, so a whole drag is one undo step; an edit that changes nothing records nothing.

## Using the editor
Capture a screenshot and press **Edit**. Tools (single key in brackets): Select (V), Pen (P),
Highlighter (H), Line (L), Arrow (A), Rectangle (R), Ellipse (E), Text (T), Marker (M), Blur (B),
Crop (C), Eraser (X).

- **Select:** click to select, drag to move, drag a handle to resize, double-click text to edit it,
  Delete removes it. The colour, width, fill, text size and blur strength controls change the selection.
- **Text:** click, type, then Ctrl+Enter or click away to finish; Esc cancels. Emptying a text removes it.
- **Marker:** each click places the next number.
- **Blur:** pixelate or blur; the export rewrites the real pixels.
- **Crop:** drag the area to keep. The original image is not changed; "Clear crop" or undo brings it back.
- **Undo/redo:** Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z. A whole drag, or a whole slider adjustment, is one step.
- **View:** Ctrl+scroll (or a pinch) to zoom around the pointer, scroll to move the picture (Shift+scroll sideways), middle-drag or Space+drag to pan, `0` fits, `1` is 100%.
- **Save edit (Ctrl+S):** writes `<output folder>/<timestamp>.guideedit`. Later saves update the same folder.
  Reopen it from the "Saved edits" tab of the main window by pasting the folder path.
- **Export image (Ctrl+E):** writes the flattened picture to the output folder in the chosen format.
- **Preview export:** renders the exact final image on a background thread and shows it.

## Known limits
- The live canvas is drawn with egui for instant feedback; the export is drawn by `guide_render`.
  Small differences (stroke ends, highlighter blending) are possible, which is what "Preview export" is for.
  The live blur is computed from the original pixels only, so it does not include annotations beneath it;
  the export and its preview do.
- Blur patches are computed on the UI thread when a blur box is created or changed. Very large
  boxes on very large images can pause the window for a moment. Preview and export are on a background thread.
- There is no file dialog: edits are saved into the output folder and opened by typing their path.
- Closing the *editor* with unsaved changes asks first. Closing the whole window does not yet.
- The eraser removes whole annotations. There is no multi-select, copy/paste, bring-to-front,
  shift-to-constrain, or numeric crop entry; a crop is changed by dragging a new one.
- Bold text is approximated (the font has no bold face).
- Only X11 has been exercised. The editor has been tested by fake mouse and keyboard at UI scales 1, 1.5 and 2
  on a virtual display, not yet by a person on a real desktop.

## Tests
`make test` (unit tests, including pixel-level renderer tests and a 300-edit undo/redo run) and
`make test-editor` (`SCALE=1.5 make test-editor`): the real binary on a virtual screen, driven by fake input.
It draws every tool, checks undo/redo, moves a shape, saves, reopens the file, exports, and checks the
annotation coordinates, the exported outline pixel, that every pixelation block is a single colour while the
same area of the original contains text, and that `original.png` is unchanged by saving again.
