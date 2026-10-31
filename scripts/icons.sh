#!/bin/sh
# Renders packaging/icons/snapforge.svg to PNGs. The PNGs are committed; rerun after editing the SVG.
set -eu
for s in 16 32 48 64 128 256; do
  mkdir -p packaging/icons/hicolor/${s}x${s}/apps
  rsvg-convert -w $s -h $s packaging/icons/snapforge.svg -o packaging/icons/hicolor/${s}x${s}/apps/snapforge.png
done
mkdir -p packaging/icons/hicolor/scalable/apps
cp packaging/icons/snapforge.svg packaging/icons/hicolor/scalable/apps/snapforge.svg
ls -l packaging/icons/hicolor/*/apps
