#!/usr/bin/env sh
# quarry installer.
#
#   curl -fsSL https://oddurs.github.io/quarry/install.sh | sh
#
# Downloads the release build for this machine, verifies its checksum, and puts
# the binary somewhere on your PATH. Set QUARRY_INSTALL_DIR to choose where.

set -eu

REPO="oddurs/quarry"
INSTALL_DIR="${QUARRY_INSTALL_DIR:-}"

die() { printf 'quarry: %s\n' "$1" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "this needs $1"; }

need curl
need tar
need uname

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Darwin) platform="apple-darwin" ;;
  Linux)  platform="unknown-linux-gnu" ;;
  *) die "no prebuilt binary for $os — install from source with: cargo install --git https://github.com/$REPO" ;;
esac

case "$arch" in
  arm64|aarch64) cpu="aarch64" ;;
  x86_64|amd64)  cpu="x86_64" ;;
  *) die "no prebuilt binary for $arch — install from source with: cargo install --git https://github.com/$REPO" ;;
esac

target="${cpu}-${platform}"

# Where to put it: the first writable directory already on PATH, else ~/.local/bin.
if [ -z "$INSTALL_DIR" ]; then
  for candidate in "$HOME/.local/bin" /usr/local/bin; do
    case ":$PATH:" in
      *":$candidate:"*) [ -w "$candidate" ] && INSTALL_DIR="$candidate" && break ;;
    esac
  done
fi
[ -n "$INSTALL_DIR" ] || INSTALL_DIR="$HOME/.local/bin"
mkdir -p "$INSTALL_DIR" || die "cannot create $INSTALL_DIR"
[ -w "$INSTALL_DIR" ] || die "$INSTALL_DIR is not writable — set QUARRY_INSTALL_DIR to somewhere that is"

printf 'quarry: looking up the latest release\n'
version="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -1)"
[ -n "$version" ] || die "could not find a release — check https://github.com/$REPO/releases"

name="quarry-${version}-${target}"
url="https://github.com/$REPO/releases/download/v${version}/${name}.tar.gz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

printf 'quarry: downloading %s\n' "$name"
curl -fsSL "$url" -o "$tmp/quarry.tar.gz" || die "could not download $url"

# Verify against the published checksums, when a hasher is available.
if curl -fsSL "https://github.com/$REPO/releases/download/v${version}/SHA256SUMS" -o "$tmp/SHA256SUMS" 2>/dev/null; then
  expected="$(grep " ${name}.tar.gz\$" "$tmp/SHA256SUMS" | awk '{print $1}' | head -1)"
  if [ -n "$expected" ]; then
    if command -v shasum >/dev/null 2>&1; then
      actual="$(shasum -a 256 "$tmp/quarry.tar.gz" | awk '{print $1}')"
    elif command -v sha256sum >/dev/null 2>&1; then
      actual="$(sha256sum "$tmp/quarry.tar.gz" | awk '{print $1}')"
    else
      actual=""
    fi
    if [ -n "$actual" ] && [ "$actual" != "$expected" ]; then
      die "checksum mismatch — refusing to install"
    fi
  fi
fi

tar -xzf "$tmp/quarry.tar.gz" -C "$tmp"
install -m 755 "$tmp/$name/quarry" "$INSTALL_DIR/quarry" 2>/dev/null \
  || { cp "$tmp/$name/quarry" "$INSTALL_DIR/quarry" && chmod 755 "$INSTALL_DIR/quarry"; }

printf 'quarry: installed %s to %s/quarry\n' "$version" "$INSTALL_DIR"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) printf 'quarry: run `quarry` to start\n' ;;
  *) printf 'quarry: %s is not on your PATH — add it, or run %s/quarry\n' "$INSTALL_DIR" "$INSTALL_DIR" ;;
esac
