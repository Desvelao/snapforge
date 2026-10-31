# Platform support

The app ships as one executable. It embeds everything it was compiled with
(Rust code, UI toolkit, fonts, image codecs, X11 protocol client) and needs no
installer or root rights. It does **not** embed the pieces that belong to the
desktop session itself.

## What the host must provide

| Needs | Why | Normally present on |
|---|---|---|
| glibc 2.35 or newer | binary built on Ubuntu 22.04 | Ubuntu 22.04+, Debian 12+, Fedora 36+, Manjaro |
| X11 client libraries: `libx11`, `libxcursor`, `libxi`, `libxrender`, `libx11-xcb`, `libxcb`, `libxkbcommon`, `libxkbcommon-x11` | the window toolkit loads them at start-up | any X11 desktop |
| OpenGL or EGL (`libgl1`/`libegl1` plus a driver) | rendering | any desktop with a GPU or Mesa |
| A font for other scripts (optional): Noto Sans CJK, Noto Sans Arabic/Hebrew/Devanagari, DejaVu Sans, ... | text in annotations, step titles and exports uses the embedded font for Latin, Greek and Cyrillic and an installed font for any other script (CJK, Arabic, Hebrew, Devanagari...); without one those letters show as empty boxes. Right-to-left order and joined (Arabic, Indic) letter shapes are not handled | most desktops |
| Wayland client libs (`libwayland-client`, `libwayland-egl`) | only for native Wayland windows | Wayland desktops |

Screen capture itself on X11 talks to the X server directly and needs none of
these libraries; only the window does. If a library is missing, the app
prints that it could not open its window and points here.

## Capture support

| Mode | X11 | Wayland |
|---|---|---|
| Full screen | yes | not implemented yet |
| Single monitor | yes | not implemented yet |
| Region (frozen-frame overlay, one monitor) | yes | not implemented yet |
| Window (hover and click on the frozen frame; includes title bar and borders) | yes, needs a window manager that provides `_NET_CLIENT_LIST_STACKING` (EWMH) | not implemented yet |
| Cursor (optional "Include cursor") | yes, needs the XFixes extension | not implemented yet |

The cursor is drawn into the full-screen image before any cropping, so region,
window and monitor captures include it too. It is captured where it is at the
moment of capture: if you start with the Capture button, it will be over that
button's position, so use the delay to move it first. If a capture asks for the
cursor and the backend cannot provide it, the capture fails with a message
instead of silently leaving it out.

On Wayland the app reports capture as unavailable instead of capturing through
XWayland, which would return blank images.

Window capture crops the frozen full-screen image, so a window partly covered
by another window is captured as it looks on screen, covered part included.
Minimized windows and windows on other virtual desktops are not offered.

## Global shortcuts

| | X11 | Wayland |
|---|---|---|
| Global shortcuts (see `docs/settings.md`) | yes | not implemented yet (needs the GlobalShortcuts portal) |
| Click capture (needs XInput2) | yes | not implemented yet |

## HiDPI and scaling

- Captures are always in **physical pixels**: a 3840x2160 screen gives a 3840x2160 image whatever the
  UI scale. The header shows the UI scale the window is using ("UI scale 2.00x").
- The UI scale comes from the desktop (RandR / `Xft.dpi`). `WINIT_X11_SCALE_FACTOR=2` forces it.
- The window opens 900x650 *points*, so it has the same physical size relative to the screen on 1x and 2x
  displays. The preview shows one image pixel per screen pixel.
- Images wider or taller than the GPU's texture limit (often 8192 or 16384, e.g. a dual-5K desktop) are
  shrunk **for display only**; the saved file keeps full resolution.
- Region and window selection work on the monitor the app window is on, using that monitor's pixels.
  On setups with different scales per monitor this is tested only by simulation (one scale at a time).

## Verified so far

`make test-hidpi` (simulated; `SCALE=1.5 GEOMETRY=3840x2160 make test-hidpi`): the real binary on a
4K virtual screen with a forced UI scale, driven by fake keys and mouse. Checks the window size in points, the
exact pixel size of full-screen, region (600x500 drag) and window captures, at scales 1, 1.5, 2 and 3.
Needs Xvnc, because winit ignores Xvfb's RandR output and always uses scale 1 there. A 17000x1000 desktop
(wider than Xvnc allows, so on Xvfb at scale 1) checks that a huge capture does not crash the preview.
This is a simulation: no physical HiDPI monitor, mixed-DPI multi-monitor setup or fractional scaling
by a real desktop environment has been tested.


`make test-x11`: global shortcuts delivered via fake key presses (also with NumLock on, not
with an extra modifier, released on drop, conflict reported as "already used"), settings loaded
and shortcuts registered through the real binary (custom and damaged files), and capture tests against a virtual X server with Openbox and an
xterm: cursor included only when requested and only near the pointer, window listing, frame size, window capture size, unknown window id.

`make portable-check` (clean Debian 12 and Ubuntu 22.04 containers, virtual X
server): headless capture self-test succeeds with no extra libraries, and the
window starts once the libraries above are installed. Individual libraries in
the table were not tested one by one, so the list may be longer than the true
minimum. Real desktops tested: XFCE on X11 (Manjaro) only.

## What has been tested

"Tested" means actually run; everything else is **not** claimed.

| System | Status |
|---|---|
| Debian 12 (container, Xvfb + software GL), x86_64 | portable check: capture self-test and a 6 s window run pass (`make portable-check`) |
| Ubuntu 22.04 (container, Xvfb + software GL), x86_64 | same, plus the whole automated suite (`make test-all`) |
| Manjaro, XFCE, X11, x86_64 (real desktop, by hand) | works |
| Fedora, openSUSE, Arch-based other than the one above, Mint, others | not tested |
| GNOME, KDE Plasma, other desktops, on X11 | not tested |
| Any Wayland session | capture and shortcuts not implemented; only the "unsupported" reporting is tested |
| aarch64 (ARM) | packages are built and checked to be aarch64 executables; **never run** |
| Fedora (latest, container), x86_64 `.rpm` | installed with its dependencies, `--self-test` passes under Xvfb, removed cleanly (`make package-check`) |
| Debian 12 and Ubuntu 22.04 (containers), x86_64 `.deb` | installed with dependencies, `--self-test` passes, removed and purged cleanly |

Minimum: glibc 2.35 (the binary is built on Ubuntu 22.04), so older distributions such as
Debian 11 or Ubuntu 20.04 are expected not to start it.
