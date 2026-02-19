#!/bin/sh
set -e

print_usage() {
  cat <<'USAGE'
Usage: install.sh [--bin-dir <dir>] [--prefix <dir>] [--version <tag>]

Install the Ata CLI (ata) from GitHub Releases.

Options:
  --bin-dir <dir>  Install ata into this directory.
  --prefix <dir>   Install into <dir>/bin (ignored if --bin-dir is set).
  --version <tag>  Release tag to install (defaults to latest). If the tag
                   does not start with "v", "v" will be prefixed.
  -h, --help       Show this help.

Examples:
  curl -fsSL https://agents2agents.ai/ata/install.sh | sh
  curl -fsSL https://agents2agents.ai/ata/install.sh | sh -s -- --bin-dir "$HOME/bin"
  curl -fsSL https://agents2agents.ai/ata/install.sh | sh -s -- --version 0.1.0
USAGE
}

BIN_DIR=""
PREFIX=""
VERSION=""

while [ $# -gt 0 ]; do
  case "$1" in
    --bin-dir)
      BIN_DIR="$2"
      shift 2
      ;;
    --prefix)
      PREFIX="$2"
      shift 2
      ;;
    --version)
      VERSION="$2"
      shift 2
      ;;
    -h|--help)
      print_usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      print_usage >&2
      exit 1
      ;;
  esac
done

if [ -n "$BIN_DIR" ] && [ -n "$PREFIX" ]; then
  echo "Only one of --bin-dir or --prefix can be specified." >&2
  exit 1
fi

if [ -z "$BIN_DIR" ]; then
  if [ -n "$PREFIX" ]; then
    BIN_DIR="$PREFIX/bin"
  else
    BIN_DIR="$HOME/.local/bin"
  fi
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required to install ata." >&2
  exit 1
fi

if ! command -v tar >/dev/null 2>&1; then
  echo "tar is required to install ata." >&2
  exit 1
fi

UNAME_S="$(uname -s)"
case "$UNAME_S" in
  Darwin)
    OS="apple-darwin"
    ;;
  Linux)
    OS="unknown-linux-musl"
    ;;
  *)
    echo "Unsupported OS: $UNAME_S" >&2
    echo "Use npm, Homebrew, or download a release from GitHub instead." >&2
    exit 1
    ;;
esac

UNAME_M="$(uname -m)"
case "$UNAME_M" in
  x86_64|amd64)
    ARCH="x86_64"
    ;;
  arm64|aarch64)
    ARCH="aarch64"
    ;;
  *)
    echo "Unsupported architecture: $UNAME_M" >&2
    exit 1
    ;;
esac

TARGET="${ARCH}-${OS}"

TAG="latest"
if [ -n "$VERSION" ]; then
  case "$VERSION" in
    rust-v*)
      TAG="$VERSION"
      ;;
    v*)
      TAG="rust-$VERSION"
      ;;
    *)
      TAG="rust-v$VERSION"
      ;;
  esac
fi

if [ "$TAG" = "latest" ]; then
  URL="https://github.com/Agents2AgentsAI/ata/releases/latest/download/ata-${TARGET}.tar.gz"
else
  URL="https://github.com/Agents2AgentsAI/ata/releases/download/${TAG}/ata-${TARGET}.tar.gz"
fi

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

ARCHIVE="$TMP_DIR/ata-${TARGET}.tar.gz"
curl -fsSL "$URL" -o "$ARCHIVE"

tar -xzf "$ARCHIVE" -C "$TMP_DIR"

SRC_BIN="$TMP_DIR/ata-${TARGET}"
if [ ! -f "$SRC_BIN" ]; then
  if [ -f "$TMP_DIR/ata" ]; then
    SRC_BIN="$TMP_DIR/ata"
  else
    echo "Unexpected archive contents." >&2
    exit 1
  fi
fi

mkdir -p "$BIN_DIR"
if command -v install >/dev/null 2>&1; then
  install -m 0755 "$SRC_BIN" "$BIN_DIR/ata"
else
  cp "$SRC_BIN" "$BIN_DIR/ata"
  chmod 0755 "$BIN_DIR/ata"
fi

echo "Installed ata to: $BIN_DIR/ata"

case ":$PATH:" in
  *":$BIN_DIR:"*)
    ;;
  *)
    echo "Add $BIN_DIR to your PATH to run 'ata' from anywhere."
    ;;
esac
