# Installing the application

The app needs no root rights to run or to install for yourself.

## Quick install (default)

```sh
curl -fsSL https://github.com/Desvelao/snapforge/releases/latest/download/get.sh | sh
```

While the program is in alpha, use the pinned form (a pre-release is not what GitHub calls "latest"):

```sh
curl -fsSL https://github.com/Desvelao/snapforge/releases/download/v0.1.0-alpha2/get.sh | sh
```

This downloads the latest release for your architecture, verifies its checksum and installs it for your
user under `~/.local` (as method 2 below). Options:

```sh
curl -fsSL https://github.com/Desvelao/snapforge/releases/latest/download/get.sh | SNAPFORGE_VERSION=0.1.0-alpha2 sh   # a specific version
curl -fsSL https://github.com/Desvelao/snapforge/releases/latest/download/get.sh | sh -s -- --prefix ~/apps --dry-run
```

Prefer to read it first? Download `get.sh`, open it, then run `sh get.sh`. Remove the program later with
`~/.local/share/snapforge/uninstall.sh`.

## Manual install

Choose one of the methods below.

### 0. Download

Packages are published on the [releases page](https://github.com/Desvelao/snapforge/releases).
Set these once in your shell, then use the commands below as written:

```sh
export SNAPFORGE_VERSION=0.1.0-alpha2     # a version listed on the releases page (tags are v<version>)
export SNAPFORGE_ARCH=x86_64       # or aarch64
export SNAPFORGE_DEB_ARCH=amd64    # .deb only: amd64, or arm64 on ARM
export SNAPFORGE_URL="https://github.com/Desvelao/snapforge/releases/download/v${SNAPFORGE_VERSION}"
```

Each method below starts by downloading the file it needs. To check a download, fetch the checksums
and verify (see [Verify the download](#verify-the-download)):

```sh
curl -fLO "$SNAPFORGE_URL/SHA256SUMS"
```

### 1. Portable (nothing is installed)

```sh
curl -fLO "$SNAPFORGE_URL/snapforge-${SNAPFORGE_VERSION}-linux-${SNAPFORGE_ARCH}.tar.gz"
tar xzf snapforge-${SNAPFORGE_VERSION}-linux-${SNAPFORGE_ARCH}.tar.gz
./snapforge-${SNAPFORGE_VERSION}-linux-${SNAPFORGE_ARCH}/snapforge
```

Move the folder anywhere, put it on a USB stick, delete it when done. Settings live in
`~/.config/snapforge/`, your guides wherever you save them.

### 2. For your user (no root)

Download and unpack the archive as in method 1, then:

```sh
cd snapforge-${SNAPFORGE_VERSION}-linux-${SNAPFORGE_ARCH}
./install.sh               # into ~/.local; use --prefix DIR for another place, --dry-run to preview
```

This copies the program, icons, menu entry and documentation under `~/.local` and records exactly
what it wrote. Remove it with `~/.local/share/snapforge/uninstall.sh`. Your settings and guides are
never deleted. If `~/.local/bin` is not on your `PATH`, start it from the menu.

### 3. System packages (root needed to install)

Download the package for your system first, then install it:

```sh
# Debian, Ubuntu
curl -fLO "$SNAPFORGE_URL/snapforge_${SNAPFORGE_VERSION}_${SNAPFORGE_DEB_ARCH}.deb"
sudo apt install ./snapforge_${SNAPFORGE_VERSION}_${SNAPFORGE_DEB_ARCH}.deb

# Fedora and similar
curl -fLO "$SNAPFORGE_URL/snapforge-${SNAPFORGE_VERSION}-1.${SNAPFORGE_ARCH}.rpm"
sudo dnf install ./snapforge-${SNAPFORGE_VERSION}-1.${SNAPFORGE_ARCH}.rpm
```

Remove with `sudo apt remove snapforge` / `sudo dnf remove snapforge`.

## Verify the download

Run this in the folder holding the downloaded files and `SHA256SUMS`:

```sh
sha256sum -c SHA256SUMS --ignore-missing
```

## What your system must provide

The program contains everything it needs except the desktop's own libraries, which every desktop
already has: glibc 2.35 or newer, X11 client libraries (`libX11`, `libXcursor`, `libXi`, `libXrender`, `libX11-xcb`, `libxcb`,
`libxkbcommon`, `libxkbcommon-x11`) and OpenGL/EGL with a driver. See
[platform-support.md](platform-support.md) for the exact list and the tested systems.
The `.deb`/`.rpm` declare these as dependencies; the portable archive and `install.sh` only warn.

Desktop notifications use `notify-send` (`libnotify-bin` on Debian and Ubuntu, `libnotify` on Fedora). It is
optional: the `.deb` recommends it, and without it the program works the same, only silently. You can set
another command in Settings > Notifications.

## Checking the installation

```sh
snapforge --version
snapforge --self-test      # needs an X11 session: one capture, settings, shortcuts
```
