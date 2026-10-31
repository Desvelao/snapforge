#!/bin/sh
# Clean-machine check: run the built binary in fresh distro containers that
# have no Rust, no dev libraries and no project files besides the binary.
#
# Stage 1 (informational): X server + software GL only. Shows which runtime
#   libraries the binary expects the desktop to provide.
# Stage 2 (must pass): plus the documented runtime libraries. The capture
#   self-test and a 6 s window run must succeed.
set -u
BIN=$(ls dist/snapforge-*-x86_64 | sort -V | tail -1)
[ -f "$BIN" ] || { echo "no binary in dist/ - run make dist first" >&2; exit 2; }
IMAGES="${IMAGES:-debian:12-slim ubuntu:22.04}"
# Keep in sync with docs/platform-support.md
RUNTIME_LIBS="libx11-6 libxcursor1 libxi6 libxrender1 libx11-xcb1 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libegl1 libgl1"
fail=0

inner() { # $1 = label, $2 = extra packages
cat <<INNER
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq --no-install-recommends xvfb xauth libgl1-mesa-dri $2 >/dev/null 2>&1
Xvfb :77 -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 &
sleep 2
export DISPLAY=:77 LIBGL_ALWAYS_SOFTWARE=1
set +e
echo "--- $1: self-test"
/snapforge --self-test; st=\$?
echo "--- $1: window (6 s)"
timeout 6 /snapforge 2>&1 | tail -3; win=\${PIPESTATUS[0]:-\$?}
echo "RESULT selftest=\$st window=\$win"
INNER
}

for img in $IMAGES; do
  echo "=================== $img"
  echo "##### stage 1: bare (expected to show missing libraries)"
  inner bare "" | docker run --rm -i -v "$PWD/$BIN:/snapforge:ro" "$img" bash -s 2>&1 | grep -E '^(---|backend|captured|capture failed|Error|RESULT)' | cut -c1-260
  echo "##### stage 2: + runtime libs (must pass)"
  out=$(inner runtime "$RUNTIME_LIBS" | docker run --rm -i -v "$PWD/$BIN:/snapforge:ro" "$img" bash -s 2>&1)
  echo "$out" | grep -E '^(---|backend|captured|capture failed|Error|RESULT)' | cut -c1-260
  echo "$out" | grep -q 'RESULT selftest=0 window=124' || { echo "FAIL: $img"; fail=1; }
done
[ "$fail" = 0 ] && echo "PORTABLE CHECK PASSED" || { echo "PORTABLE CHECK FAILED"; exit 1; }
