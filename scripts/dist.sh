#!/bin/sh
# Builds the size-optimised release binaries and enforces the size budget.
# ARCHES defaults to x86_64 only; `make package` builds both x86_64 and aarch64.
set -eu
BUDGET_BYTES=$((15 * 1024 * 1024))
ARCHES=${ARCHES:-x86_64}
version=$(cargo pkgid -p guide-app | sed 's/.*[#@]//')
mkdir -p dist
for arch in $ARCHES; do
  case "$arch" in
    x86_64) cargo build --release -p guide-app; built="${CARGO_TARGET_DIR:-target}/release/snapforge" ;;
    aarch64) cargo build --release -p guide-app --target aarch64-unknown-linux-gnu
             built="${CARGO_TARGET_DIR:-target}/aarch64-unknown-linux-gnu/release/snapforge" ;;
    *) echo "unknown architecture $arch" >&2; exit 2 ;;
  esac
  out="dist/snapforge-${version}-${arch}"
  # Copy then rename: replacing a binary that is currently running fails with cp alone.
  cp "$built" "$out.tmp"
  mv -f "$out.tmp" "$out"
  size=$(stat -c %s "$out")
  echo "$out: $size bytes (budget $BUDGET_BYTES)"
  [ "$size" -le "$BUDGET_BYTES" ] || { echo "size budget exceeded" >&2; exit 1; }
done
