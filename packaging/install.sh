#!/bin/sh
# Installs the application for the current user. Needs no root and writes only under the prefix
# (default ~/.local). Run it from the unpacked archive.
#   ./install.sh [--prefix DIR] [--dry-run] [--help]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
prefix="${HOME:?HOME is not set}/.local"
dry=0
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) [ $# -ge 2 ] || { echo "--prefix needs a folder" >&2; exit 64; }; prefix=$2; shift 2 ;;
    --dry-run) dry=1; shift ;;
    -h|--help)
      sed -n '2,4p' "$0" | sed 's/^# \{0,1\}//'
      echo "  --prefix DIR  install under DIR instead of ~/.local"
      echo "  --dry-run     show what would be done, change nothing"
      exit 0 ;;
    *) echo "unknown option: $1 (try --help)" >&2; exit 64 ;;
  esac
done
case "$prefix" in /*) ;; *) echo "--prefix must be an absolute path" >&2; exit 64 ;; esac
[ -x "$here/snapforge" ] || { echo "snapforge binary not found next to install.sh" >&2; exit 1; }

manifest="$prefix/share/snapforge/install-manifest.txt"
files=$(cd "$here" && find share -type f | sort)

echo "Installing snapforge into $prefix"
run() { if [ "$dry" = 1 ]; then echo "  would: $*"; else "$@"; fi; }

run mkdir -p "$prefix/bin" "$prefix/share/snapforge"
run install -m 755 "$here/snapforge" "$prefix/bin/snapforge"
list="$prefix/bin/snapforge"
for f in $files; do
  dest="$prefix/$f"
  run mkdir -p "$(dirname "$dest")"
  if [ "$f" = share/applications/snapforge.desktop ]; then
    # The menu entry must find the program even when ~/.local/bin is not on PATH.
    if [ "$dry" = 1 ]; then echo "  would: write $dest with Exec=$prefix/bin/snapforge"; else
      sed "s|^Exec=.*|Exec=$prefix/bin/snapforge|" "$here/$f" > "$dest"; fi
  else
    run install -m 644 "$here/$f" "$dest"
  fi
  list="$list
$dest"
done
for doc in README.md LICENSE THIRD-PARTY-LICENSES.txt; do
  [ -f "$here/$doc" ] || continue
  run install -m 644 "$here/$doc" "$prefix/share/snapforge/$doc"
  list="$list
$prefix/share/snapforge/$doc"
done
if [ -d "$here/docs" ]; then
  for d in $(cd "$here" && find docs -type f | sort); do
    run mkdir -p "$(dirname "$prefix/share/snapforge/$d")"
    run install -m 644 "$here/$d" "$prefix/share/snapforge/$d"
    list="$list
$prefix/share/snapforge/$d"
  done
fi
if [ "$dry" = 1 ]; then echo "Dry run: nothing was changed."; exit 0; fi
printf '%s\n%s\n' "$list" "$manifest" > "$manifest"

command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" 2>/dev/null || true
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$prefix/share/applications" 2>/dev/null || true

echo "Installed. Start it from the applications menu or run: $prefix/bin/snapforge"
case ":$PATH:" in *":$prefix/bin:"*) ;; *) echo "Note: $prefix/bin is not on your PATH; use the menu entry or the full path." ;; esac
echo "Remove it later with: $prefix/share/snapforge/uninstall.sh"
# The uninstaller travels with the install, so it works after the archive is deleted.
install -m 755 "$here/uninstall.sh" "$prefix/share/snapforge/uninstall.sh"
echo "$prefix/share/snapforge/uninstall.sh" >> "$manifest"

# Libraries the desktop must provide (see docs/platform-support.md).
missing=""
for lib in libX11.so.6 libXcursor.so.1 libXi.so.6 libXrender.so.1 libX11-xcb.so.1 libxcb.so.1 libxkbcommon.so.0 libxkbcommon-x11.so.0 libEGL.so.1 libGL.so.1; do
  if command -v ldconfig >/dev/null 2>&1 && ! ldconfig -p 2>/dev/null | grep -q "$lib"; then missing="$missing $lib"; fi
done
[ -z "$missing" ] || echo "Warning: these libraries were not found:$missing. the application may not open its window until they are installed (see docs/install.md)."
exit 0
