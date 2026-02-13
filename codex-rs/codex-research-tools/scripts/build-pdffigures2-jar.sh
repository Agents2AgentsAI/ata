#!/usr/bin/env bash
#
# Build the pdffigures2 fat JAR using Docker (no host JDK/SBT required).
#
# Usage:
#   ./build-pdffigures2-jar.sh [output_path]
#
# The resulting JAR will be written to:
#   <output_path>  (default: ./pdffigures2.jar)
#
# Prerequisites: Docker must be installed and running.

set -euo pipefail

OUTPUT="${1:-pdffigures2.jar}"

REPO_URL="https://github.com/allenai/pdffigures2.git"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

echo "==> Cloning pdffigures2 into $WORKDIR …"
git clone --depth 1 "$REPO_URL" "$WORKDIR/pdffigures2"

echo "==> Building fat JAR via Docker …"
docker run --rm \
  -v "$WORKDIR/pdffigures2:/app" \
  -w /app \
  mozilla/sbt:8u292_1.5.7 \
  sbt assembly

# build.sbt sets: assembly / assemblyOutputPath := file("pdffigures2.jar")
# so the fat JAR lands in the project root, not under target/.
JAR="$WORKDIR/pdffigures2/pdffigures2.jar"

if [ ! -f "$JAR" ]; then
  echo "ERROR: assembly JAR not found at $JAR" >&2
  echo "Listing project root:" >&2
  ls -la "$WORKDIR/pdffigures2/" >&2
  exit 1
fi

cp "$JAR" "$OUTPUT"
echo "==> JAR written to $OUTPUT ($(du -h "$OUTPUT" | cut -f1))"
