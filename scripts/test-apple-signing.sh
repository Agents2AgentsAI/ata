#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SECRETS_FILE="${REPO_ROOT}/secrets"
CERTS_DIR="$HOME/.apple-certs"
KEYCHAIN_PATH="/tmp/test-signing.keychain-db"
KEYCHAIN_PASS="temppass"
BINARY="${REPO_ROOT}/codex-rs/target/release/ata"
ZIP_PATH="/tmp/ata-notarize.zip"

# Load secrets
if [[ ! -f "$SECRETS_FILE" ]]; then
  echo "Error: secrets file not found at $SECRETS_FILE"
  exit 1
fi
set -a && source "$SECRETS_FILE" && set +a

# Clean up any previous test keychain
security delete-keychain "$KEYCHAIN_PATH" 2>/dev/null || true

# Import cert into temporary keychain
echo "==> Importing certificate into temporary keychain..."
security create-keychain -p "$KEYCHAIN_PASS" "$KEYCHAIN_PATH"
security import "$CERTS_DIR/developer_id_application_compat.p12" \
  -k "$KEYCHAIN_PATH" \
  -P "$APPLE_CERTIFICATE_PASSWORD" \
  -T /usr/bin/codesign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple: -s -k "$KEYCHAIN_PASS" "$KEYCHAIN_PATH" > /dev/null
security list-keychains -s "$KEYCHAIN_PATH" $(security list-keychains | tr -d '"')

# Find signing identity
IDENTITY=$(security find-identity -v -p codesigning "$KEYCHAIN_PATH" \
  | sed -n 's/.*\([0-9A-F]\{40\}\).*/\1/p' \
  | head -1)

if [[ -z "$IDENTITY" ]]; then
  echo "Error: no signing identity found"
  security delete-keychain "$KEYCHAIN_PATH"
  exit 1
fi
echo "==> Signing identity: $IDENTITY"

# Sign the binary
echo "==> Signing binary..."
codesign --force --options runtime --timestamp \
  --keychain "$KEYCHAIN_PATH" \
  --sign "$IDENTITY" \
  "$BINARY"

echo "==> Verifying signature..."
codesign -dvv "$BINARY" 2>&1

# Submit for notarization
echo "==> Submitting for notarization (this may take several minutes)..."
rm -f "$ZIP_PATH"
ditto -c -k --keepParent "$BINARY" "$ZIP_PATH"

xcrun notarytool submit "$ZIP_PATH" \
  --key "$CERTS_DIR"/AuthKey_*.p8 \
  --key-id "$KEY_ID" \
  --issuer "$APPLE_NOTARIZATION_ISSUER_ID" \
  --wait

# Cleanup
echo "==> Cleaning up..."
security delete-keychain "$KEYCHAIN_PATH"
rm -f "$ZIP_PATH"

echo "==> Done!"
