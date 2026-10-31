#!/bin/sh
# Line coverage of the whole test suite: the unit and integration tests, then the X11 tests
# (live capture, hotkeys and the end-to-end window tests that run the real binary on a virtual
# display). Writes dist/coverage/summary.txt and an HTML report in dist/coverage/html.
set -eu
mkdir -p dist/coverage
cargo llvm-cov clean --workspace
cargo llvm-cov nextest --no-report --workspace --no-tests=pass

Xvnc :81 -geometry 2560x1600 -depth 24 -SecurityTypes None -localhost yes >/dev/null 2>&1 &
sleep 2
export DISPLAY=:81
openbox >/dev/null 2>&1 &
sleep 1
xterm -T snapforge-live -geometry 100x40+60+60 -bg black -fg white \
  -e sh -c 'yes "The quick brown fox jumps over the lazy dog 0123456789 sensitive" | head -80; sleep 100000' >/dev/null 2>&1 &
sleep 2
export SNAPFORGE_EXPECT_WINDOWS=1 SNAPFORGE_E2E=1 SNAPFORGE_E2E_SCALE=1
# Fake input is global to the X server, so these run one at a time.
cargo llvm-cov nextest --no-report -j1 --no-tests=pass \
  -p guide-capture --test x11_live \
  -p guide-hotkeys --test x11_live \
  -p guide-app --test guide_e2e --test composition_e2e --test export_e2e \
  --test a11y_e2e --test workflow_e2e --test gui_e2e --test editor_e2e

cargo llvm-cov report --html --output-dir dist/coverage
cargo llvm-cov report --summary-only | tee dist/coverage/summary.txt
