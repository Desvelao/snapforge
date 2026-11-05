# Troubleshooting

**The window does not open ("could not open its window").** A desktop library is missing. Install
the libraries listed in [platform-support.md](platform-support.md) (on Debian/Ubuntu:
`sudo apt install libx11-6 libxcursor1 libxi6 libxrender1 libx11-xcb1 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libegl1 libgl1`).
With no GPU, install Mesa (`libgl1-mesa-dri`); `LIBGL_ALWAYS_SOFTWARE=1 snapforge` forces software drawing.

**"Capture unavailable" or black/empty images on Wayland.** Wayland capture is not implemented.
Log in to an X11 session, or run a desktop on X11. The app refuses to capture through XWayland
because that returns blank images.

**A shortcut shows "already used by another application".** Another program (often the desktop's
own screenshot tool) owns that key. Choose another combination under Shortcuts. Shortcuts need a
modifier key and do not work on Wayland.

**Window capture lists no windows.** The window manager must support the EWMH
`_NET_CLIENT_LIST_STACKING` property. Full-screen and region capture still work.

**The cursor is not in the screenshot.** Turn on "Include cursor". It needs the XFixes X extension.
If you start with the Capture button the cursor is over that button; use a delay or a shortcut.

**Click capture says it is not available.** It needs an X11 session whose server has the XInput2
extension; on Wayland it is not implemented. Clicks on the app window itself are not captured
because the window is minimized during a session.

**Settings were reset.** If `~/.config/snapforge/settings.json` could not be read it is renamed to
`settings.json.bad` and defaults are used; a message says so. Fix and rename it back to recover.

**"Your changes are NOT saved".** The guide folder cannot be written (disk full, read-only, removed).
Free space or fix permissions and press "Try again". Nothing is deleted while saving fails.

**A guide will not open or shows problems.** The app recovers a damaged `project.json` from its
`.bak` copy and keeps the damaged file. `snapforge --check <folder.snapforgeproj>` lists problems without
a display: exit 0 = fine, 1 = problems (such as missing screenshots), 2 = cannot be read.

**Text is too small or large.** Settings > Appearance, or Ctrl+Plus / Ctrl+Minus / Ctrl+0.

**Reporting a problem.** Include `snapforge --version`, `snapforge --self-test` output, your
distribution, desktop and whether the session is X11 or Wayland (`echo $XDG_SESSION_TYPE`).

**`snapforge gui` does nothing / exits at once.** Run it in a terminal: it prints why (for example
"capture is not available" on Wayland, exit code 1; bad options exit 64). It needs an X11 session.

**After Enter, nothing is in the clipboard.** The image is held by a background process
(`snapforge --serve-clipboard`) until you copy something else; check it is running with
`pgrep -fa serve-clipboard`. If a clipboard manager replaces the clipboard with something empty,
disable that behaviour in the manager.

**The overlay appears on the wrong monitor.** Multi-monitor placement is untested; the overlay is
meant to open on the monitor under the pointer and falls back to the primary one.

## No notifications appear

Notifications run the command under Settings > Notifications, by default `notify-send` from libnotify.
Press "Send a test notification" there: it shows when the program is missing or exits with an error.
Install `libnotify-bin` (Debian, Ubuntu) or `libnotify` (Fedora, Arch), make sure a notification daemon
is running, or set another command such as `dunstify {title} {message}`.

## The file dialog does not open

"Save as file" in `snapforge gui` and the "Browse..." buttons use the system file dialog through the
desktop portal (`xdg-desktop-portal` plus a backend such as `-gtk`, `-kde` or `-gnome`). Without
one running, "Save as file" saves a timestamped file into the screenshots folder instead, and
"Browse..." does nothing: type the path into the field.
