#!/bin/sh
# End-to-end test of the annotation editor: real binary, fake mouse and keyboard.
# Usage: test-editor.sh [scale]
set -eu
SCALE=${1:-1}
# The window is 1100x720 points, so at a UI scale above 1 the virtual screen must be bigger than the window.
GEOMETRY=1920x1080
if awk "BEGIN { exit !($SCALE > 1) }"; then GEOMETRY=2560x1600; fi
Xvnc :79 -geometry $GEOMETRY -depth 24 -SecurityTypes None -localhost yes >/dev/null 2>&1 &
sleep 2
export DISPLAY=:79
openbox >/dev/null 2>&1 &
sleep 1
# An xterm full of text, so there is real detail for the blur test to hide.
xterm -T snapforge-live -geometry 100x40+60+60 -bg black -fg white \
  -e sh -c 'yes "The quick brown fox jumps over the lazy dog 0123456789 sensitive" | head -80; sleep 100000' >/dev/null 2>&1 &
sleep 2
mkdir -p dist/e2e
SNAPFORGE_E2E=1 SNAPFORGE_E2E_SCALE="$SCALE" SNAPFORGE_E2E_SHOT_DIR=/work/dist/e2e cargo nextest run -p guide-app --test editor_e2e --no-capture
