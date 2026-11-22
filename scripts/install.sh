#!/usr/bin/env bash
# Installs the docair command-line tool from its GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/agyss/docair/master/scripts/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/agyss/docair/master/scripts/install.sh | bash -s -- --version 0.1.0
#
# Options:
#   --version X.Y.Z   install that release (default: the latest release)
#   --dir DIR         install into DIR (default: $HOME/.local/bin)
#
# Each release has, per platform, docair-<version>-<os>-<arch>.tar.gz (holding
# docair) and SHA256SUMS. The script checks the checksum before it installs.
# Platforms: linux-x86_64 and macos-arm64. Windows: the zip on the release page.

set -euo pipefail

repo=agyss/docair
# Where release files live. Overridable for a local mirror or a test.
download_base=${DOCAIR_RELEASE_URL:-https://github.com/$repo/releases/download}
version=""
dir=${DOCAIR_INSTALL_DIR:-$HOME/.local/bin}

usage() {
  echo "usage: install.sh [--version X.Y.Z] [--dir DIR]"
}

fail() {
  echo "docair install: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version) [ $# -ge 2 ] || fail "--version needs a value"; version=$2; shift 2 ;;
    --version=*) version=${1#*=}; shift ;;
    --dir) [ $# -ge 2 ] || fail "--dir needs a value"; dir=$2; shift 2 ;;
    --dir=*) dir=${1#*=}; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; fail "unknown option: $1" ;;
  esac
done

# --- platform ---------------------------------------------------------------
case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  *) fail "no build for $(uname -s); Windows: download the zip from https://github.com/$repo/releases" ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=arm64 ;;
  *) fail "no build for $(uname -m)" ;;
esac
target="$os-$arch"
case "$target" in
  linux-x86_64|macos-arm64) ;;
  *) fail "no build for $target (builds: linux-x86_64, macos-arm64)" ;;
esac

command -v curl >/dev/null 2>&1 || fail "curl is required"
tar --version >/dev/null 2>&1 || fail "tar is required"

# --- version ---------------------------------------------------------------
if [ -z "$version" ]; then
  # GitHub redirects /releases/latest to the newest tag's page; the tag is its last segment.
  latest=$(curl -sSL -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest") \
    || fail "could not reach GitHub"
  case "$latest" in
    */releases/tag/*) tag=${latest##*/} ;;
    *) fail "no published release yet (see https://github.com/$repo/releases); use --version once one exists" ;;
  esac
  version=${tag#v}
fi
version=${version#v}
# X.Y.Z, or a pre-release: X.Y.Z-alpha, X.Y.Z-beta-2
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta)(-[0-9]+)?)?$ ]]; then
  fail "not a version: $version (expected X.Y.Z, X.Y.Z-alpha or X.Y.Z-beta-N)"
fi

# --- download and verify -----------------------------------------------------
asset="docair-$version-$target.tar.gz"
url="$download_base/v$version"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -fsSL -o "$tmp/$asset" "$url/$asset" \
  || fail "release v$version has no $asset (check the version and target)"
curl -fsSL -o "$tmp/SHA256SUMS" "$url/SHA256SUMS" \
  || fail "release v$version has no SHA256SUMS"

expected=$(awk -v f="$asset" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")
[ -n "$expected" ] || fail "SHA256SUMS lists no $asset"
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$tmp/$asset" | awk '{ print $1 }')
else
  actual=$(shasum -a 256 "$tmp/$asset" | awk '{ print $1 }')
fi
[ "$actual" = "$expected" ] || fail "checksum mismatch for $asset; nothing installed"

tar -xzf "$tmp/$asset" -C "$tmp"
[ -f "$tmp/docair" ] || fail "$asset does not contain docair"

# --- install ---------------------------------------------------------------
mkdir -p "$dir"
install -m 0755 "$tmp/docair" "$dir/docair"
echo "installed docair $version ($target) to $dir/docair"

case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "note: $dir is not on your PATH; add it, e.g.  export PATH=\"$dir:\$PATH\"" ;;
esac
