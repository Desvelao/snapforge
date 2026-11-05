# Exporting a project

Status: both export modes are implemented and tested (`guide-export`, driven by the Export section and the
**Export (Ctrl+E)** button of the guide window).

## Two modes
- **One image per step.** Each *included* step becomes its own file.
- **One combined image (guides and diffs).** The whole guide is laid out as a single picture by the composition engine
  (see `docs/composition.md`): title, introduction, numbered steps with titles and descriptions, final notes.

A **diff** is exported as one combined image of its previous and post screenshot, side by side or stacked, with the
optional header and the captions under the screenshots.

Steps with "Include in export" switched off are never exported in either mode.

## File names
| Mode | Name | Example |
|---|---|---|
| Separate, "01-title" | number and sanitized title | `01-open-settings.png`, `02-pick-an-option.png` |
| Separate, "01" | number only | `01.png`, `02.png` |
| Combined | sanitized guide title | `how-to-change-a-setting.png` |

- The number is the step's position among the **included** steps, the same number drawn on its badge. Numbers are
  padded to two digits (three from 100 steps) so the files sort in guide order.
- Titles are reduced to lower-case ASCII letters, digits and single dashes (at most 50 characters); everything else,
  including `/`, `\`, `..` and non-ASCII characters, is dropped, so a name is valid on every file system and can never
  point outside the output folder. A title with nothing usable left becomes `step` (guide: `guide`).
- The extension follows the format: `.png`, `.jpg` (JPEG), `.webp` (WebP, lossless).
- The Export section lists the file names before you export.

## Where files go, and what happens to existing files
- The output folder defaults to the guide's own `exports/` folder and can be changed; it is created if missing.
- Existing files are **never overwritten by default**: a clash writes `01-open-settings-2.png` (then `-3`...) beside it.
- With **Replace files with the same name** on, each file is written completely to a temporary file and swapped in with a
  rename, so an old file is only ever replaced by a whole new one.
- Nothing is ever deleted. The guide folder is not modified by exporting.

## Options
- **Format:** PNG, JPEG or WebP (lossless). JPEG has no transparency.
- **Separate mode, "Draw the number, title and description on each image":** each file is the screenshot with its badge,
  title and description laid out around it with the guide's composition settings (font size, colours, margins), at the
  screenshot's natural size, numbered with the step's real number. Off: the file is just the finished screenshot
  (annotations burned in, crop applied).
- Annotations are always burned in and the crop applied; blur/pixelate rewrite the real pixels.

## Progress, cancelling, and problems
- Exporting runs on a background thread; the window stays responsive and shows a progress bar with a Cancel button.
  Cancelling stops before the next step's work starts; files already written are kept; nothing partial is left behind.
- The result lists every file written (path and size) and every problem. A step that cannot be exported (missing or
  unreadable screenshot) is reported with its number and title and the other steps are still exported. In the combined
  image a missing screenshot is drawn as a grey placeholder **and** reported, never silently.
- An output folder that cannot be used, or a file that cannot be written, is reported with its path.
- If no step is included, nothing is written and the reason is shown.

## Size limits
The combined image is limited to 120 million pixels. The size is computed before anything is drawn; if it is too large
nothing is written and the message offers the alternatives: export the steps as separate images, use a narrower content
width, or use fewer steps.

## Limits
- Export settings (mode, format, folder, options) are kept while the guide is open, not saved in the guide file.
- Non-ASCII letters in titles are dropped from file names (a title in Japanese becomes `01-step.png`); use "01.png" names if
  that matters.
- Exporting works on the guide as it is in memory; screenshots are read from the project folder at export time.
- Only X11 has been exercised, by fake mouse and keyboard at UI scales 1, 1.5 and 2, not yet by a person on a real desktop.

## Tests
`make test` covers file naming, both modes, order, annotations and crops, every format, never-overwrite and replace,
each failure path, the size limit, cancelling, progress and that the project is untouched, all without any window.
`make test-guide` drives the real window: export with the button and with Ctrl+E, switch modes, export twice (the second
run must not overwrite the first), delete a screenshot and check the problem is reported while the other steps are written.
