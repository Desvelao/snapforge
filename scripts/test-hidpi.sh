#!/bin/sh
# Simulated HiDPI end-to-end test: a 4K virtual screen (Xvnc, because winit ignores
# Xvfb's RandR output and would always use scale 1.0), the real binary with a
# forced UI scale factor, fake keyboard/mouse input. Usage: test-hidpi.sh [scale] [WIDTHxHEIGHT]
set -eu
SCALE=${1:-2}
GEOMETRY=${2:-3840x2160}
WIDTH=${GEOMETRY%x*}
if [ "$WIDTH" -gt 16000 ]; then
  # Wider than Xvnc allows. Xvfb has no usable RandR output, so scale is 1 here, and a
  # single monitor this wide cannot host the fullscreen overlay (GPU framebuffer limit),
  # so only the full-screen capture + preview (the texture-size path) is exercised.
  Xvfb :78 -screen 0 "${GEOMETRY}x24" -nolisten tcp >/dev/null 2>&1 &
  export SNAPFORGE_E2E_FULLSCREEN_ONLY=1
else
  Xvnc :78 -geometry "$GEOMETRY" -depth 24 -SecurityTypes None -localhost yes >/dev/null 2>&1 &
fi
sleep 2
export DISPLAY=:78
openbox >/dev/null 2>&1 &
sleep 1
xterm -T snapforge-live -geometry 80x24+300+200 >/dev/null 2>&1 &
sleep 2
mkdir -p dist/e2e
SNAPFORGE_E2E=1 SNAPFORGE_E2E_SCALE="$SCALE" SNAPFORGE_E2E_SHOT_DIR=/work/dist/e2e \
  cargo nextest run -p guide-app --test hidpi_e2e --no-capture
