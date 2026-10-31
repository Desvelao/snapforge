#!/bin/sh
# Writes the packaging files that show the display name into dist/render/, with @APP_NAME@
# replaced by the name in app.toml (the one file that spells it).
set -eu
name=$(sed -n 's/^name[[:space:]]*=[[:space:]]*"\(.*\)"[[:space:]]*$/\1/p' app.toml | head -n 1)
version=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\(.*\)"[[:space:]]*$/\1/p' app.toml | head -n 1)
[ -n "$version" ] || { echo "app.toml has no version" >&2; exit 1; }
[ -n "$name" ] || { echo "app.toml has no name" >&2; exit 1; }
mkdir -p dist/render
for f in snapforge.desktop snapforge.metainfo.xml; do
  sed -e "s|@APP_NAME@|$name|g" -e "s|@APP_VERSION@|$version|g" "packaging/$f" > "dist/render/$f"
done
