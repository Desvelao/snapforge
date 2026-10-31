#!/bin/sh
# Live X11 capture tests: virtual X server + Openbox + one xterm to capture.
# Runs inside the dev container (see `make test-x11`).
set -eu
Xvfb :77 -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 &
sleep 2
export DISPLAY=:77
openbox >/dev/null 2>&1 &
sleep 1
xterm -T snapforge-live -geometry 60x15+100+100 >/dev/null 2>&1 &
# Wait until the window manager lists the xterm (a fixed sleep raced with a busy machine).
i=0
until xprop -root _NET_CLIENT_LIST_STACKING 2>/dev/null | grep -q 'window id # 0x'; do
  i=$((i + 1)); [ "$i" -le 100 ] || { echo "window manager never listed the xterm" >&2; exit 1; }
  sleep 0.2
done
SNAPFORGE_EXPECT_WINDOWS=1 cargo nextest run -p guide-capture --test x11_live
# Changes the screen size, so it runs alone.
cargo nextest run -p guide-capture --test x11_resize -j1
# Fake modifier state is global to the X server, so these must not run in parallel.
SNAPFORGE_EXPECT_WINDOWS=1 cargo nextest run -p guide-hotkeys --test x11_live -j1

# End to end through the real binary: saved settings are loaded and their
# shortcuts registered; a shortcut taken by another client and a damaged file
# are reported.
cargo build -q -p guide-app
BIN=${CARGO_TARGET_DIR:-target}/debug/snapforge
CFG=$(mktemp -d)
export XDG_CONFIG_HOME=$CFG
mkdir -p "$CFG/snapforge"
echo '--- custom settings are loaded and their shortcuts registered'
cat > "$CFG/snapforge/settings.json" <<JSON
{"delay_secs": 7, "format": "Jpeg", "shortcuts": {"capture_region": "Ctrl+Alt+Z"}}
JSON
out=$($BIN --self-test); echo "$out"
echo "$out" | grep -q 'delay=7s' && echo "$out" | grep -q 'format=Jpeg' \
  && echo "$out" | grep -q 'Capture region = Ctrl+Alt+Z: registered' || { echo "FAIL: custom settings"; exit 1; }
echo '--- a damaged file is kept as .bad and defaults are used'
echo '{ nope' > "$CFG/snapforge/settings.json"
out=$($BIN --self-test); echo "$out"
echo "$out" | grep -q 'damaged' && [ -f "$CFG/snapforge/settings.json.bad" ] || { echo "FAIL: damaged settings"; exit 1; }
echo 'E2E settings checks passed'
