#!/bin/sh
# Installs the built artifacts in clean distro containers and checks that they work and leave
# nothing behind. Run `make package` first. Needs network access for the distro images.
#   1. tarball + install.sh as an unprivileged user (ubuntu:22.04), dry run, install, run, uninstall
#   2. .deb install/remove/purge on ubuntu:22.04 and debian:12
#   3. .rpm install/remove on fedora
#   4. the aarch64 files are checked as ELF aarch64; they are NOT executed (no emulation here)
set -u
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
D="$PWD/dist"
fail=0
check() { # label, image, script
  echo "=================== $1 ($2)"
  if docker run --rm -v "$D:/dist:ro" "$2" sh -c "$3" 2>&1 | tail -n 40 | tee /tmp/pkgcheck.out | grep -q "^PACKAGE-CHECK-OK$"; then
    grep -E '^(---|snapforge|OK|backend|captured)' /tmp/pkgcheck.out; echo "RESULT $1: PASS"
  else
    cat /tmp/pkgcheck.out; echo "RESULT $1: FAIL"; fail=1
  fi
}

XVFB='Xvfb :77 -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 & sleep 2; export DISPLAY=:77 LIBGL_ALWAYS_SOFTWARE=1'

check "tarball, unprivileged install" ubuntu:22.04 "set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq --no-install-recommends xvfb xauth libgl1-mesa-dri libx11-6 libxcursor1 libxi6 libxkbcommon0 libxkbcommon-x11-0 libegl1 libgl1 >/dev/null 2>&1
useradd -m tester
cat > /home/tester/t.sh <<'INNER'
set -e
cd /tmp && tar xzf /dist/snapforge-${version}-linux-x86_64.tar.gz && cd snapforge-${version}-linux-x86_64
(cd /tmp/snapforge-${version}-linux-x86_64 && sha256sum -c SHA256SUMS >/dev/null) && echo '--- checksums ok'
./snapforge --version
./install.sh --dry-run >/dev/null
[ ! -e \$HOME/.local/bin/snapforge ] || { echo 'dry run installed something'; exit 1; }
echo '--- dry run changed nothing'
./install.sh >/dev/null
for f in bin/snapforge share/applications/snapforge.desktop share/icons/hicolor/256x256/apps/snapforge.png share/metainfo/snapforge.metainfo.xml share/snapforge/uninstall.sh share/snapforge/docs/install.md; do
  [ -e \$HOME/.local/\$f ] || { echo missing \$f; exit 1; }
done
grep -q \"^Exec=\$HOME/.local/bin/snapforge\" \$HOME/.local/share/applications/snapforge.desktop || { echo 'Exec not rewritten'; exit 1; }
echo '--- installed'
$XVFB
\$HOME/.local/bin/snapforge --self-test
mkdir -p \$HOME/.config/snapforge \$HOME/keepme && echo '{}' > \$HOME/.config/snapforge/settings.json
\$HOME/.local/share/snapforge/uninstall.sh >/dev/null
left=\$(find \$HOME/.local -type f | wc -l)
[ \"\$left\" = 0 ] || { find \$HOME/.local -type f; echo 'files left behind'; exit 1; }
[ -f \$HOME/.config/snapforge/settings.json ] || { echo 'settings were deleted'; exit 1; }
echo '--- uninstalled, nothing left, settings kept'
INNER
su tester -c 'sh /home/tester/t.sh'
echo PACKAGE-CHECK-OK"

for img in ubuntu:22.04 debian:12-slim; do
check "deb" $img "set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq --no-install-recommends xvfb xauth libgl1-mesa-dri >/dev/null 2>&1
apt-get install -y -qq --no-install-recommends /dist/snapforge_${version}_amd64.deb >/dev/null 2>&1 || { apt-get install -y /dist/snapforge_${version}_amd64.deb; exit 1; }
echo '--- installed with its dependencies'
snapforge --version
$XVFB
snapforge --self-test
desktop-file-validate /usr/share/applications/snapforge.desktop 2>/dev/null || true
apt-get remove -y -qq snapforge >/dev/null 2>&1
[ ! -e /usr/bin/snapforge ] || { echo 'binary left after remove'; exit 1; }
apt-get purge -y -qq snapforge >/dev/null 2>&1 || true   # nothing left to purge: apt says so
[ ! -e /usr/share/applications/snapforge.desktop ] && [ ! -d /usr/share/doc/snapforge ] || { echo 'files left after purge'; exit 1; }
echo '--- removed and purged cleanly'
echo PACKAGE-CHECK-OK"
done

check "rpm" fedora:latest "set -e
dnf install -y -q xorg-x11-server-Xvfb mesa-dri-drivers >/dev/null 2>&1
dnf install -y -q /dist/snapforge-${version}-1.x86_64.rpm >/dev/null 2>&1 || { dnf install -y /dist/snapforge-${version}-1.x86_64.rpm; exit 1; }
echo '--- installed with its dependencies'
snapforge --version
$XVFB
snapforge --self-test
dnf remove -y -q snapforge >/dev/null 2>&1
[ ! -e /usr/bin/snapforge ] || { echo 'binary left after remove'; exit 1; }
echo '--- removed cleanly'
echo PACKAGE-CHECK-OK"

echo "=================== aarch64 (not executed)"
for f in dist/snapforge-${version}-aarch64 ; do
  if file "$f" | grep -q "ELF 64-bit LSB.*ARM aarch64"; then echo "RESULT aarch64 binary: PASS (ELF aarch64, not run)"; else file "$f"; fail=1; fi
done
docker compose run --rm test dpkg-deb -f "dist/snapforge_${version}_arm64.deb" Architecture 2>/dev/null | grep -qx arm64 && echo "RESULT arm64 deb: PASS (architecture field, not installed)" || fail=1
[ "$fail" = 0 ] && echo "PACKAGE CHECK PASSED" || { echo "PACKAGE CHECK FAILED"; exit 1; }
