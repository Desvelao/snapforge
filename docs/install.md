# Installing the application

The app needs no root rights to run or to install for yourself. Choose one:

## 1. Portable (nothing is installed)

```sh
tar xzf snapforge-0.1.0-linux-x86_64.tar.gz
./snapforge-0.1.0-linux-x86_64/snapforge
```

Move the folder anywhere, put it on a USB stick, delete it when done. Settings live in
`~/.config/snapforge/`, your guides wherever you save them.

## 2. For your user (no root)

```sh
cd snapforge-0.1.0-linux-x86_64
./install.sh               # into ~/.local; use --prefix DIR for another place, --dry-run to preview
```

This copies the program, icons, menu entry and documentation under `~/.local` and records exactly
what it wrote. Remove it with `~/.local/share/snapforge/uninstall.sh`. Your settings and guides are
never deleted. If `~/.local/bin` is not on your `PATH`, start it from the menu.

## 3. System packages (root needed to install)

```sh
sudo apt install ./snapforge_0.1.0_amd64.deb         # Debian, Ubuntu (use _arm64 on ARM)
sudo dnf install ./snapforge-0.1.0-1.x86_64.rpm      # Fedora and similar (aarch64 on ARM)
```

Remove with `sudo apt remove snapforge` / `sudo dnf remove snapforge`.

## Verify the download

```sh
sha256sum -c SHA256SUMS --ignore-missing
```

## What your system must provide

The program contains everything it needs except the desktop's own libraries, which every desktop
already has: glibc 2.35 or newer, X11 client libraries (`libX11`, `libXcursor`, `libXi`, `libXrender`, `libX11-xcb`, `libxcb`,
`libxkbcommon`, `libxkbcommon-x11`) and OpenGL/EGL with a driver. See
[platform-support.md](platform-support.md) for the exact list and the tested systems.
The `.deb`/`.rpm` declare these as dependencies; the portable archive and `install.sh` only warn.

## Checking the installation

```sh
snapforge --version
snapforge --self-test      # needs an X11 session: one capture, settings, shortcuts
```
