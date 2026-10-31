#!/bin/sh
# End-to-end test of the guide window: real binary, fake mouse and keyboard.
# Usage: test-guide.sh [scale]
set -eu
SCALE=${1:-1}
# The window is 1100x720 points, so at a UI scale above 1 the virtual screen must be bigger than the window.
GEOMETRY=1920x1080
if awk "BEGIN { exit !($SCALE > 1) }"; then GEOMETRY=2560x1600; fi
Xvnc :80 -geometry $GEOMETRY -depth 24 -SecurityTypes None -localhost yes >/dev/null 2>&1 &
sleep 2
export DISPLAY=:80
openbox >/dev/null 2>&1 &
sleep 2
mkdir -p dist/e2e
SNAPFORGE_E2E=1 SNAPFORGE_E2E_SCALE="$SCALE" SNAPFORGE_E2E_SHOT_DIR=/work/dist/e2e cargo nextest run -p guide-app -j1 --test guide_e2e --test composition_e2e --test export_e2e --test a11y_e2e --test workflow_e2e --test gui_e2e --test click_e2e --no-capture
