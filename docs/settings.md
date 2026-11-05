# Settings and shortcuts

## Where settings live
`$XDG_CONFIG_HOME/snapforge/settings.json`, or `~/.config/snapforge/settings.json`
when `XDG_CONFIG_HOME` is not set. The file is plain JSON:

```json
{
  "version": 1,
  "out_dir": "/home/me/Pictures/<app name>",
  "format": "Png",
  "delay_secs": 0,
  "include_cursor": false,
  "mode": "FullScreen",
  "shortcuts": {
    "capture_full_screen": "Ctrl+Shift+F",
    "capture_region": "Ctrl+Shift+R",
    "capture_window": "Ctrl+Shift+W",
    "toggle_click_capture": "Ctrl+Shift+C",
    "repeat_capture": "Ctrl+Shift+A"
  },
  "click_marker": false,
  "click_max": 0,
  "profiles": [
    {
      "name": "Default",
      "click_capture_options": { "marker": false, "max": 0 },
      "editor_options": { "color": [230, 40, 40, 255], "width": 4.0, "text_size": 28.0 }
    }
  ],
  "active_profile": "Default"
}
```

The example is shortened: `language`, `theme`, `recent_projects` and `recent_edits` are described
under "Appearance and recent items" below.

`format` is `Png`, `Jpeg` or `WebP`. `mode` is `FullScreen`, `Monitor`, `Region` or `Window`.

## Behaviour
- Changes are saved about half a second after the last edit, and again when the window closes.
- Writes are atomic (temp file, then rename), so a crash cannot leave a half-written file.
- A missing file gives defaults. Unknown fields are ignored; missing fields use defaults.
- A file that is not JSON (or is larger than 4 MB) is **not deleted**: it is renamed to
  `settings.json.bad` (`.bad.1`, `.bad.2`... if one is already there), defaults are used, and the
  window shows a message with the new name.
- Invalid or duplicated shortcuts are reset to their defaults, with a message.
- A saved mode that does not work on the current machine (for example "Single monitor" with
  one monitor, or "Window" without a window manager that lists windows) falls back to Full screen
  for that run. The file is not changed until you change something.
- A saved value that this version cannot read (for example a new image format name added by a
  newer version) is reset on its own; every other setting is kept and a message names the ones
  that were reset.
- A file saved by a newer version is copied to `settings.json.v<N>` the first time it is read, because
  saving writes only the settings this version knows.

## Global shortcuts
Defaults: Ctrl+Shift+F (full screen), Ctrl+Shift+R (region), Ctrl+Shift+W (window),
Ctrl+Shift+C (start or stop click capture), Ctrl+Shift+A (repeat the last capture).
Print is not used by default because desktops such as XFCE already bind it.

- A shortcut needs at least one of Ctrl, Alt, Shift or Super, so it cannot steal ordinary typing.
- Keys: letters, digits, F1-F12, Print, Space, Return, Tab, arrows, Home, End, PageUp, PageDown,
  Insert, Delete.
- Change them under the gear button (Settings > Shortcuts): click the shortcut, press the new keys.
  The window can record Ctrl, Alt and Shift combinations; Super and Print can only be set by
  editing the file (for example `"Super+Print"`).
- If another application already owns a shortcut, it shows "already used by another application"
  and does nothing until you pick a different one.
- Shortcuts work in any application, but only while the app is running.
- X11 only. On Wayland, apps cannot grab keys; the window says so. This needs the
  GlobalShortcuts portal, which is not implemented yet.
- NumLock and CapsLock do not stop shortcuts from working.
- Holding a shortcut down starts only one capture.

## Appearance and recent items

| Field | Meaning |
|---|---|
| `language` | `""` = automatic; otherwise a language code (see `docs/i18n.md`). |
| `theme` | `System`, `Light` or `Dark`. |
| `recent_projects`, `recent_edits` | The last 10 folders opened, newest first, without duplicates. Folders that no longer exist are listed as missing with a "Forget" button. |

## Capture

| Field | Meaning |
|---|---|
| `restore_after_capture` | `true` (default): the window comes back after a capture. `false`: it stays minimized while captures are added to an open project ("Add captures to the project"), so you can keep capturing with the shortcuts. A region or window pick still shows the selection screen first and minimizes again afterwards. Turn on the notification for captures to get feedback while the window is hidden. |

## Capture

| Field | Meaning |
|---|---|
| `restore_after_capture` | `true` (default): the window comes back after a capture. `false`: it stays minimized while captures are added to an open project ("Add captures to the project"), so you can keep capturing with the shortcuts. A region or window pick still shows the selection screen first and minimizes again afterwards. Turn on the notification for captures to get feedback while the window is hidden. |

## Notifications

| Field | Meaning |
|---|---|
| `notifications.enabled` | `true` (default): announce a finished copy, image save, edit save, project save and export. Autosave is never announced. |
| `notifications.on_capture` | `true` (default): also announce each screenshot taken. Needs `enabled`. |
| `notifications.command` | The program to run, default `notify-send --app-name=Snapforge {title} {message}`. |

The command is split into words the way a shell would (single and double quotes, backslash) but is **not**
run by a shell: no pipes, no `$(...)`, no variables. `{title}` and `{message}` are replaced inside each
word, so a file name with odd characters stays one argument. An empty or unbalanced command is reset to the
default when the settings are read. Settings > Notifications has a "Send a test notification" button that
shows whether the command ran and what it printed if it failed. A failing command is otherwise ignored.

## Click capture
A session started with Ctrl+Shift+C (or the "Click capture" button) takes a screenshot at every left
click until the same shortcut is pressed again, Esc is pressed in the focused window, or `click_max`
shots were taken (`0` means no limit, at most 1000). The main window is minimized while it runs.

- What a click captures follows the capture mode: full screen, the single monitor, or the window under
  the click. Region mode captures the full screen, because there is nobody to drag a region.
- With a project open each shot becomes a new step after the selected one; otherwise it is saved in
  the output folder like any capture.
- `click_marker` (off by default) puts a numbered marker annotation on each new step at the click.
  The marker is an ordinary annotation you can move or delete; the stored screenshot is not changed.
- Clicks within 250 ms of each other count once. The scroll wheel and other buttons are ignored.
- X11 only: it listens through the XInput2 extension. Elsewhere the button reports why it is unavailable.
- `snapforge clicks [--count N] [--path DIR]` runs a session without a window and prints each saved file.

## Profiles
A profile is a named set of tool settings: `click_capture_options` (`marker`, `max`, as in Click capture
above) and `editor_options` (the editor's colour, widths, fill, text size, bold, blur mode and strength,
and marker size). `active_profile` names the one used now and is selected again on the next start.

- Pick, create, rename and delete profiles next to the "Click capture" button or in the editor's tool bar.
  A new profile is a copy of the current one. The last profile cannot be deleted; there can be up to 20,
  with names of up to 40 characters, unique ignoring case.
- Changing "Mark clicks", "Max shots" or a tool setting changes the active profile.
- The numbered markers added by click capture use the profile's colour and width.
- `click_marker` and `click_max` at the top level mirror the active profile for older builds. When a file
  has no `profiles`, a "Default" profile is made from them.
