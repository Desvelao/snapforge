#!/bin/sh
# One-line installer: downloads a release, checks its checksum and installs it for the current user
# (no root). Usage:
#   curl -fsSL https://github.com/Desvelao/snapforge/releases/latest/download/get.sh | sh
#   curl -fsSL .../get.sh | sh -s -- --prefix DIR --dry-run     (extra options go to install.sh)
# Environment:
#   SNAPFORGE_VERSION  version to install (default: the latest release)
#   SNAPFORGE_ARCH     x86_64 or aarch64 (default: this machine)
#   SNAPFORGE_URL      base URL of the releases (default: the GitHub project), for mirrors and tests
set -eu

REPO_URL="${SNAPFORGE_URL:-https://github.com/Desvelao/snapforge/releases}"

say() { printf '%s\n' "$*"; }
err() { printf '❌ %s\n' "$*" >&2; }
die() { err "$*"; exit 1; }

fetch() { # fetch URL DEST
  if command -v curl >/dev/null 2>&1; then curl -fsSL -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then wget -q -O "$2" "$1"
  else die "curl or wget is required"; fi
}

latest_version() {
  tag=""
  if command -v curl >/dev/null 2>&1; then
    tag=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$REPO_URL/latest" 2>/dev/null) || tag=""
  else
    tag=$(wget -q --spider -S "$REPO_URL/latest" 2>&1 | sed -n 's/^ *[Ll]ocation: *//p' | tail -n 1) || tag=""
  fi
  tag=${tag##*/}
  case "$tag" in v[0-9]*) printf '%s\n' "${tag#v}"; return 0 ;; esac
  # "latest" ignores pre-releases; fall back to the newest release of any kind (GitHub API).
  api=${SNAPFORGE_API_URL:-https://api.github.com/repos/Desvelao/snapforge/releases?per_page=1}
  fetch "$api" "$tmp_api" 2>/dev/null || return 1
  tag=$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp_api" | head -n 1)
  [ -n "$tag" ] || return 1
  printf '%s\n' "${tag#v}"
}

main() {
  [ "$(uname -s)" = Linux ] || die "this installer supports Linux only"
  for tool in tar sha256sum mktemp; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is required"
  done

  arch="${SNAPFORGE_ARCH:-$(uname -m)}"
  case "$arch" in
    x86_64|amd64) arch=x86_64 ;;
    aarch64|arm64) arch=aarch64 ;;
    *) die "unsupported architecture: $arch (x86_64 and aarch64 are provided)" ;;
  esac

  version="${SNAPFORGE_VERSION:-}"
  if [ -z "$version" ]; then
    say "🔍 Looking for the latest release..."
    tmp_api=$(mktemp)
    version=$(latest_version) && [ -n "$version" ] || die "could not find the latest release at $REPO_URL"
    rm -f "$tmp_api"
  fi
  version=${version#v}

  name="snapforge-$version-linux-$arch"
  base="$REPO_URL/download/v$version"
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT INT TERM

  say "⬇️  Downloading $name.tar.gz"
  fetch "$base/$name.tar.gz" "$tmp/$name.tar.gz" || die "download failed: $base/$name.tar.gz"
  fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "download failed: $base/SHA256SUMS"

  say "🔐 Verifying the checksum"
  (cd "$tmp" && grep -F " $name.tar.gz" SHA256SUMS | sha256sum -c - >/dev/null 2>&1) \
    || die "checksum mismatch or missing entry for $name.tar.gz; nothing was installed"

  say "📦 Unpacking"
  tar -xzf "$tmp/$name.tar.gz" -C "$tmp" || die "could not unpack the archive"
  [ -x "$tmp/$name/install.sh" ] || die "install.sh not found in the archive"

  say "🚀 Installing"
  (cd "$tmp/$name" && ./install.sh "$@") || die "installation failed"
  say "✅ Done. Check it with: snapforge --version"
}

# Everything runs from main so a truncated download never executes half a script.
main "$@"
