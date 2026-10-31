# Screenshot guide

Capture screenshots, annotate them, and keep several of them in a project. A project can be a plain set of
screenshots, or a numbered step-by-step guide that you can export as separate images or one long image.

* **Capture** the whole screen, one monitor, a region or a window, with an optional cursor, a delay
  and global keyboard shortcuts (X11 only for now).
* **Annotate** with pen, highlighter, line, arrow, rectangle, ellipse, text, numbered markers, blur
  and crop. Every edit can be undone; the original screenshot is never changed.
* **Organise projects**: reorder, duplicate and exclude screenshots, give each a title, description and
  caption, export them as separate images. Make the project a **guide** to add numbering, an introduction,
  final notes, a layout preview and the one-long-image export.
* **Portable**: one executable, no installer and no root rights needed. Your projects are plain
  folders (`.snapforgeproj`) with PNG images and a JSON file.

## Quick capture without the window (`snapforge gui`)

```sh
snapforge gui                 # freeze the screen, drag a region, annotate
snapforge gui --delay 3000    # wait 3 s first (milliseconds)
snapforge gui --path ~/Pictures/shots
```

No main window opens. Drag a region, the editor opens on it, then **Enter** (or Ctrl+C) copies the
image to the clipboard and quits, **Ctrl+S** saves a file and quits, **Esc** cancels. The clipboard
keeps the image after the app has exited. Bind the command to a key, for example on XFCE under
Settings > Keyboard > Application Shortcuts (add `snapforge gui`, or the full path if you did not
install it). X11 only.

## One screenshot per click

`Ctrl+Shift+C` (or the **Click capture** button) starts a session: every left click takes a screenshot
until you press the shortcut again. With a guide or project open each click becomes a step, optionally
with a marker at the click point. `snapforge clicks --count 5 --path ~/Pictures/shots` does it without
a window. See `docs/settings.md`. X11 only.

## Get it

Install for your user (no root) with one command; it downloads the latest release, checks its checksum
and installs it:

```sh
curl -fsSL https://github.com/Desvelao/snapforge/releases/latest/download/get.sh | sh
```

Or do it by hand: download `snapforge-<version>-linux-<arch>.tar.gz` (x86_64 or aarch64) from the
[releases page](https://github.com/Desvelao/snapforge/releases), then:

```sh
export SNAPFORGE_VERSION=0.1.0 SNAPFORGE_ARCH=x86_64   # see the releases page; or aarch64
curl -fLO "https://github.com/Desvelao/snapforge/releases/download/v${SNAPFORGE_VERSION}/snapforge-${SNAPFORGE_VERSION}-linux-${SNAPFORGE_ARCH}.tar.gz"
tar xzf snapforge-${SNAPFORGE_VERSION}-linux-${SNAPFORGE_ARCH}.tar.gz
cd snapforge-${SNAPFORGE_VERSION}-linux-${SNAPFORGE_ARCH}
./snapforge                 # run it right here, nothing is installed
./install.sh                # or: install for your user only (no root), adds a menu entry
```

`.deb` and `.rpm` packages are provided too. See [docs/install.md](docs/install.md).

**Status:** Capture and global shortcuts work on **X11** sessions. **Wayland capture is
not implemented yet.** Read [docs/platform-support.md](docs/platform-support.md) for what has and has
not been tested before relying on it.

## Documentation

* [Installing](docs/install.md) and [troubleshooting](docs/troubleshooting.md)
* [Platform support and tested systems](docs/platform-support.md)
* [Settings](docs/settings.md), [annotations](docs/annotations.md), [project format](docs/project-format.md),
  [composition](docs/composition.md), [export](docs/export.md), [languages and accessibility](docs/i18n.md)
* [Testing](docs/testing.md)

## Building from source

Everything is built in Docker; no Rust toolchain is needed on the host.

```sh
make test          # unit and integration tests
make lint
make dist          # release binary in dist/ (size budget 15 MB)
make package       # tarball, .deb and .rpm for x86_64 and aarch64
make test-all      # the full release gate
```

## Licence

MIT, see [LICENSE](LICENSE). Third-party licences: [THIRD-PARTY-LICENSES.txt](THIRD-PARTY-LICENSES.txt).
