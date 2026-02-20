#!/usr/bin/env bash
#
# Test that pdffigures2.jar can be built from source on macOS and Ubuntu.
#
# Usage:
#   ./test-pdffigures2-build.sh            # run both macOS + Ubuntu (Docker)
#   ./test-pdffigures2-build.sh macos      # macOS only
#   ./test-pdffigures2-build.sh ubuntu     # Ubuntu (Docker) only
#
# Prerequisites:
#   macOS:  brew, openjdk@11 (or @8), sbt
#   Ubuntu: docker

set -euo pipefail

PASS=0
FAIL=0

pass() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# ---------------------------------------------------------------------------
# Well-known JAVA_HOME locations (mirrors find_build_jdk() in pdf_images.rs)
# ---------------------------------------------------------------------------
BUILD_JDK_HOMES=(
    /opt/homebrew/opt/openjdk@11
    /usr/local/opt/openjdk@11
    /opt/homebrew/opt/openjdk@8
    /usr/local/opt/openjdk@8
    /usr/lib/jvm/java-11-openjdk-amd64
    /usr/lib/jvm/java-11-openjdk-arm64
    /usr/lib/jvm/java-8-openjdk-amd64
    /usr/lib/jvm/java-8-openjdk-arm64
)

find_build_jdk() {
    for home in "${BUILD_JDK_HOMES[@]}"; do
        if [ -f "$home/bin/java" ]; then
            echo "$home"
            return 0
        fi
    done
    return 1
}

# ---------------------------------------------------------------------------
# macOS test
# ---------------------------------------------------------------------------
test_macos() {
    echo "=== macOS: pdffigures2 build test ==="

    # 1. Find a compatible JDK
    if ! JAVA_HOME=$(find_build_jdk); then
        fail "no JDK 11 or 8 found at well-known paths"
        echo "       Install with: brew install openjdk@11"
        return 1
    fi
    pass "found build JDK at $JAVA_HOME"
    "$JAVA_HOME/bin/java" -version 2>&1 | head -1

    # 2. Check SBT
    if ! command -v sbt &>/dev/null; then
        fail "sbt not found on PATH"
        echo "       Install with: brew install sbt"
        return 1
    fi
    pass "sbt found at $(command -v sbt)"

    # 3. Clone + build
    WORKDIR="$(mktemp -d)"
    trap 'rm -rf "$WORKDIR"' RETURN

    echo "  Cloning pdffigures2..."
    git clone --depth 1 https://github.com/allenai/pdffigures2.git "$WORKDIR/pdffigures2" 2>&1 | tail -1
    pass "git clone succeeded"

    echo "  Running sbt assembly (JAVA_HOME=$JAVA_HOME)..."
    cd "$WORKDIR/pdffigures2"
    if ! JAVA_HOME="$JAVA_HOME" sbt assembly 2>&1 | tail -3; then
        fail "sbt assembly failed"
        return 1
    fi
    pass "sbt assembly completed"

    # 4. Validate JAR
    JAR="$WORKDIR/pdffigures2/pdffigures2.jar"
    if [ ! -f "$JAR" ]; then
        fail "pdffigures2.jar not found in project root"
        ls -la "$WORKDIR/pdffigures2/"
        return 1
    fi

    SIZE=$(stat -f%z "$JAR" 2>/dev/null || stat --printf="%s" "$JAR" 2>/dev/null)
    if [ "$SIZE" -gt 1000000 ]; then
        pass "pdffigures2.jar built successfully ($SIZE bytes)"
    else
        fail "pdffigures2.jar too small ($SIZE bytes)"
        return 1
    fi

    echo "=== macOS: ALL PASSED ==="
}

# ---------------------------------------------------------------------------
# Ubuntu test (via Docker)
# ---------------------------------------------------------------------------
test_ubuntu() {
    echo "=== Ubuntu: pdffigures2 build test (Docker) ==="

    if ! command -v docker &>/dev/null; then
        fail "docker not found on PATH"
        return 1
    fi

    docker run --rm ubuntu:22.04 bash -c '
set -euo pipefail

PASS=0
FAIL=0
pass() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); exit 1; }

echo "--- Installing prerequisites ---"
apt-get update -qq > /dev/null
apt-get install -y -qq curl gnupg apt-transport-https git > /dev/null 2>&1
pass "base packages installed"

# Install SBT (mirrors ensure_sbt() apt path)
echo "deb https://repo.scala-sbt.org/scalasbt/debian all main" \
    | tee /etc/apt/sources.list.d/sbt.list > /dev/null
curl -sL "https://keyserver.ubuntu.com/pks/lookup?op=get&search=0x2EE0EA64E40A89B84B2DF73499E82A75642AC823" \
    | apt-key add - 2>/dev/null
apt-get update -qq > /dev/null 2>&1
apt-get install -y -qq sbt > /dev/null 2>&1
pass "sbt installed"

# Install JDK 11 (mirrors find_build_jdk() apt path)
apt-get install -y -qq openjdk-11-jdk-headless > /dev/null 2>&1
pass "openjdk-11 installed"

JAVA_HOME=/usr/lib/jvm/java-11-openjdk-amd64
if [ ! -d "$JAVA_HOME" ]; then
    JAVA_HOME=/usr/lib/jvm/java-11-openjdk-arm64
fi
export JAVA_HOME
"$JAVA_HOME/bin/java" -version 2>&1 | head -1

echo ""
echo "--- Clone + build ---"
git clone --depth 1 https://github.com/allenai/pdffigures2.git /tmp/pdffigures2 2>&1 | tail -1
pass "git clone succeeded"

cd /tmp/pdffigures2
echo "  Running sbt assembly (JAVA_HOME=$JAVA_HOME)..."
if ! sbt assembly 2>&1 | tail -3; then
    fail "sbt assembly failed"
fi
pass "sbt assembly completed"

JAR=/tmp/pdffigures2/pdffigures2.jar
if [ ! -f "$JAR" ]; then
    fail "pdffigures2.jar not found"
fi

SIZE=$(stat --printf="%s" "$JAR")
if [ "$SIZE" -gt 1000000 ]; then
    pass "pdffigures2.jar built successfully ($SIZE bytes)"
else
    fail "pdffigures2.jar too small ($SIZE bytes)"
fi

echo ""
echo "=== Ubuntu: ALL PASSED ($PASS checks) ==="
'
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
TARGET="${1:-all}"

case "$TARGET" in
    macos)  test_macos ;;
    ubuntu) test_ubuntu ;;
    all)
        test_macos
        echo ""
        test_ubuntu
        ;;
    *)
        echo "Usage: $0 [macos|ubuntu|all]"
        exit 1
        ;;
esac

echo ""
echo "========================================="
echo " Results: $PASS passed, $FAIL failed"
echo "========================================="
[ "$FAIL" -eq 0 ] || exit 1
