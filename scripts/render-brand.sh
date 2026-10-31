#!/bin/sh
# Writes the packaging files that show the display name into dist/render/, with @APP_NAME@
# replaced by the name in brand.toml (the one file that spells it).
set -eu
name=$(sed -n 's/^name[[:space:]]*=[[:space:]]*"\(.*\)"[[:space:]]*$/\1/p' brand.toml | head -n 1)
[ -n "$name" ] || { echo "brand.toml has no name" >&2; exit 1; }
mkdir -p dist/render
for f in snapforge.desktop snapforge.metainfo.xml; do
  sed "s|@APP_NAME@|$name|g" "packaging/$f" > "dist/render/$f"
done
