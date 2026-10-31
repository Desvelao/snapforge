# Composition: laying out a guide

Status: the layout engine (`guide_render::compose`) and the three-panel guide window are implemented
and tested. Writing the result to files is described in `docs/export.md`; the window shows exactly what an export
will contain.

## The three panels
- **Left: steps.** Thumbnails with numbers and titles, drag-and-drop reordering, excluded and broken steps marked.
- **Centre: composition preview.** The whole guide as one picture, with its output size
  ("Output size: 864 x 1500 px"). Click a step to select it (it gets a blue outline and is selected in the list).
  Scroll to move, Ctrl+scroll to zoom, **Fit** to fit the width, **100%** for one picture pixel per screen pixel.
  **Final-size preview** draws the exact image at full size instead of the quick scaled-down one.
- **Right: properties.** Guide title, introduction, final notes; the selected step (title, description,
  include, per-step overrides); layout and image settings; numbering and text; background, spacing, margins.

Either side panel can be folded away with the **Steps** / **Properties** buttons above the preview. On a window narrower
than 900 points the properties start folded. The capture controls fold into one row while a guide is open.

## One layout for preview and export
`compute_layout` works out every position, size and wrapped line of text in output pixels, and the final image
size, **before anything is drawn**. `render_layout` then draws that layout at any `scale`. The quick preview is the same
layout drawn at about 1000 px wide; the final-size preview and the export draw it at scale 1. Text is wrapped
once, from the unscaled measurements, so both show the same line breaks.

## Rules
- **Width.** The image is `content width + 2 x margin` wide. Content width is a setting (default 1200 px).
- **Vertical layout.** One step per row, in guide order, `spacing` apart. Excluded steps are left out and numbering
  counts only the included ones (1, 2, 3...).
- **Grid layout.** 1 to 8 columns; each cell is `(content width - gaps) / columns` wide; a row is as tall as its tallest cell.
- **Each step** is: number badge and title, then description, then screenshot (or screenshot, then description if
  placed below).
- **Screenshots** keep their aspect ratio. *Shrink to fit* shrinks wide ones to the slot and leaves small ones at their
  natural size; *fill the width* scales every one to the slot width. A step may set its own maximum image width.
  Narrower images are aligned left, centre or right. Shrinking averages pixels, so text stays readable.
- **Text** wraps at word boundaries inside its slot; over-long words are broken; paragraph breaks are kept; nothing is
  truncated or dropped. Step titles are 1.25x the base font size, the guide title 1.8x.
- **Optional parts.** Guide title, introduction, final notes, step numbers, titles and descriptions can each be shown or
  hidden; a step can hide or force its own description.
- **A missing screenshot** is drawn as a labelled grey placeholder instead of failing the whole guide.
- **Size limit.** The result must be under 120 million pixels. The size is computed first; if it is too big the window and
  the engine say so and suggest exporting the steps as separate images, a narrower content width, or fewer steps. The
  final-size preview is further limited to 40 megapixels.

## Limits
- Preview rendering and annotation drawing run on a background thread; very long guides draw only what is needed, and
  the quick preview keeps at most 64 shrunken step images in memory.
- The preview shows a step's annotated screenshot; changing annotations updates it after a short pause.
- Only X11 has been exercised, by fake mouse and keyboard at UI scales 1, 1.5 and 2, not yet by a person on a real desktop.

## Tests
`make test` covers the engine without any window (wrapping, scaling modes, alignment, grid, toggles, size limit,
same layout at two scales, placeholders) and `make test-guide` drives the real window: the size shown equals the size the
engine computes, clicking a step selects it, panels fold, typing a content width and toggling descriptions change the output
by exactly what the engine predicts, the final-size preview uses the same layout, and the settings are saved.
