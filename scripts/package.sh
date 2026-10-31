#!/bin/sh
# Builds the release artifacts for x86_64 and aarch64 into dist/:
#   snapforge-<ver>-linux-<arch>.tar.gz   portable archive with a no-root installer
#   snapforge_<ver>_<amd64|arm64>.deb
#   snapforge-<ver>-1.<x86_64|aarch64>.rpm
#   SHA256SUMS
set -eu
export ARCHES="x86_64 aarch64"
TARGET_DIR=${CARGO_TARGET_DIR:-target}
version=$(cargo pkgid -p guide-app | sed 's/.*[#@]//')

echo "--- validating metadata"
sh scripts/render-brand.sh
desktop-file-validate dist/render/snapforge.desktop
# The component id is not reverse-DNS yet; the two findings that follow from that are the only ones allowed.
out=$(appstreamcli validate --no-net dist/render/snapforge.metainfo.xml 2>&1 || true)
echo "$out" | grep -E '^[EWIP]:' | grep -v -e cid-desktopapp-is-not-rdns -e cid-contains-hyphen && { echo "$out"; echo "metainfo problems" >&2; exit 1; }
sh scripts/licenses.sh >/dev/null

sh scripts/dist.sh

for arch in x86_64 aarch64; do
  case $arch in x86_64) deb=amd64; triple="" ;; aarch64) deb=arm64; triple="aarch64-unknown-linux-gnu" ;; esac
  bin="dist/snapforge-${version}-${arch}"

  echo "--- tarball ($arch)"
  name="snapforge-${version}-linux-${arch}"
  stage="dist/stage/$name"
  rm -rf "dist/stage/$name"; mkdir -p "$stage/share/applications" "$stage/share/metainfo" "$stage/docs"
  cp "$bin" "$stage/snapforge"; chmod 755 "$stage/snapforge"
  cp packaging/install.sh packaging/uninstall.sh "$stage/"
  cp -r packaging/icons/hicolor "$stage/share/icons-tmp" && mkdir -p "$stage/share/icons" && mv "$stage/share/icons-tmp" "$stage/share/icons/hicolor"
  cp dist/render/snapforge.desktop "$stage/share/applications/"
  cp dist/render/snapforge.metainfo.xml "$stage/share/metainfo/"
  cp LICENSE README.md THIRD-PARTY-LICENSES.txt CHANGELOG.md "$stage/"
  cp docs/*.md "$stage/docs/"
  mkdir -p "$stage/licenses"; cp crates/guide-render/assets/UFL.txt crates/guide-app/assets/fonts/Inter-LICENSE.txt crates/guide-app/assets/fonts/JetBrainsMono-OFL.txt "$stage/licenses/"
  (cd "$stage" && find . -type f | sort | sed 's|^\./||' | xargs sha256sum > SHA256SUMS)
  # Fixed owner and sorted names, so the same input gives the same archive.
  tar --owner=0 --group=0 --numeric-owner --sort=name -C dist/stage -czf "dist/$name.tar.gz" "$name"

  echo "--- deb ($arch)"
  if [ -n "$triple" ]; then
    cargo deb -p guide-app --no-build --no-strip --target "$triple" --output "dist/snapforge_${version}_${deb}.deb"
  else
    cargo deb -p guide-app --no-build --no-strip --output "dist/snapforge_${version}_${deb}.deb"
  fi

  echo "--- rpm ($arch)"
  if [ -n "$triple" ]; then
    cargo generate-rpm -p crates/guide-app --target "$triple" --target-dir "$TARGET_DIR" --arch "$arch" -o "dist/snapforge-${version}-1.${arch}.rpm"
  else
    cargo generate-rpm -p crates/guide-app --target-dir "$TARGET_DIR" --arch "$arch" -o "dist/snapforge-${version}-1.${arch}.rpm"
  fi
done
rm -rf dist/stage
(cd dist && sha256sum snapforge-"${version}"-* snapforge_"${version}"_* > SHA256SUMS)
ls -l dist/*.tar.gz dist/*.deb dist/*.rpm
