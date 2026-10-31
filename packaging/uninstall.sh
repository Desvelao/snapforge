#!/bin/sh
# Removes what install.sh installed, using the list it recorded. Your settings
# (~/.config/snapforge) and your guides and screenshots are never touched.
#   uninstall.sh [--prefix DIR]
set -eu
prefix="${HOME:?HOME is not set}/.local"
here=$(cd "$(dirname "$0")" && pwd)
# Run from the installed copy: share/snapforge/uninstall.sh -> prefix is two levels up.
case "$here" in */share/snapforge) prefix=${here%/share/snapforge} ;; esac
[ "${1:-}" = "--prefix" ] && [ $# -ge 2 ] && prefix=$2
manifest="$prefix/share/snapforge/install-manifest.txt"
[ -f "$manifest" ] || { echo "No snapforge installation recorded under $prefix" >&2; exit 1; }
while IFS= read -r f; do
  case "$f" in "$prefix"/*) rm -f -- "$f" ;; esac
done < "$manifest"
# Remove folders this install left empty, deepest first, but never the prefix itself.
for d in share/snapforge/docs share/snapforge share/icons/hicolor/scalable/apps share/icons/hicolor/scalable \
  share/icons/hicolor/16x16/apps share/icons/hicolor/32x32/apps share/icons/hicolor/48x48/apps \
  share/icons/hicolor/64x64/apps share/icons/hicolor/128x128/apps share/icons/hicolor/256x256/apps \
  share/icons/hicolor/16x16 share/icons/hicolor/32x32 share/icons/hicolor/48x48 share/icons/hicolor/64x64 \
  share/icons/hicolor/128x128 share/icons/hicolor/256x256 share/metainfo; do
  rmdir "$prefix/$d" 2>/dev/null || true
done
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$prefix/share/applications" 2>/dev/null || true
echo "snapforge was removed from $prefix. Your settings and guides were left alone."
