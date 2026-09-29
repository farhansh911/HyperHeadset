#!/bin/bash
# Build HyperHeadset.app for Apple Silicon and install it in /Applications.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script only builds the macOS app." >&2
  exit 1
fi

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "This script targets Apple Silicon (arm64). This Mac is $(uname -m)." >&2
  exit 1
fi

# A stable signing identity is what lets macOS remember device access.
# Ad-hoc signatures get a new identity often enough that the prompt returns
# every time the app is opened.
KEYCHAIN="${HOME}/Library/Keychains/hyperheadset-signing.keychain-db"
KEYCHAIN_PASS="hyperheadset-local"
CERT_NAME="HyperHeadset Local"

ensure_signing_identity() {
  if [[ -f "$KEYCHAIN" ]] && security find-identity -p codesigning -v "$KEYCHAIN" 2>/dev/null | grep -q "$CERT_NAME"; then
    security unlock-keychain -p "$KEYCHAIN_PASS" "$KEYCHAIN"
    return
  fi

  echo "Creating a local signing identity (one time)..."
  local tmp
  tmp="$(mktemp -d)"
  cat > "$tmp/cert.conf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = ${CERT_NAME}
[ext]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
EOF
  openssl req -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes \
    -keyout "$tmp/key.pem" -out "$tmp/cert.pem" \
    -config "$tmp/cert.conf" -extensions ext >/dev/null 2>&1
  # OpenSSL 3's default PKCS#12 MAC is rejected by `security import`.
  openssl pkcs12 -export -inkey "$tmp/key.pem" -in "$tmp/cert.pem" \
    -out "$tmp/cert.p12" -name "$CERT_NAME" -passout "pass:${KEYCHAIN_PASS}" \
    -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg SHA1 >/dev/null 2>&1

  security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || true
  security create-keychain -p "$KEYCHAIN_PASS" "$KEYCHAIN"
  security set-keychain-settings -lut 21600 "$KEYCHAIN"
  security unlock-keychain -p "$KEYCHAIN_PASS" "$KEYCHAIN"
  security import "$tmp/cert.p12" -k "$KEYCHAIN" -P "$KEYCHAIN_PASS" \
    -T /usr/bin/codesign -T /usr/bin/security >/dev/null
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
    -k "$KEYCHAIN_PASS" "$KEYCHAIN" >/dev/null
  # Trust it for code signing so macOS can keep the privacy grant.
  security add-trusted-cert -d -r trustAsRoot -p codeSign -k "$KEYCHAIN" "$tmp/cert.pem" >/dev/null 2>&1 \
    || security add-trusted-cert -d -r trustRoot -p codeSign -k "$KEYCHAIN" "$tmp/cert.pem"
  rm -rf "$tmp"

  if ! security list-keychains -d user | grep -q "hyperheadset-signing"; then
    # shellcheck disable=SC2046
    security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
  fi
}

echo "Building..."
cargo build --release

VERSION="$(awk -F\" '/^version = / { print $2; exit }' Cargo.toml)"
# Cursor's sandbox sets CARGO_TARGET_DIR. A normal Terminal build uses ./target.
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BIN="$TARGET_DIR/release/hyper_headset"
if [[ ! -x "$BIN" ]]; then
  echo "Release binary not found at $BIN" >&2
  exit 1
fi
APP="/Applications/HyperHeadset.app"
# The Dock Apps folder is /Applications. A Desktop copy never shows up there.
rm -rf "${HOME}/Desktop/HyperHeadset.app"

if [[ ! -w "/Applications" ]]; then
  echo "Cannot write to /Applications. Install from an admin account." >&2
  exit 1
fi

osascript -e 'tell application "HyperHeadset" to quit' >/dev/null 2>&1 || true
killall HyperHeadset >/dev/null 2>&1 || true

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN" "$APP/Contents/MacOS/HyperHeadset"
chmod +x "$APP/Contents/MacOS/HyperHeadset"
sed "s/__VERSION__/${VERSION}/g" "$ROOT/macos/Info.plist" > "$APP/Contents/Info.plist"

echo "Drawing icon..."
ICON_DIR="$(mktemp -d)"
ICON_PNG="$ICON_DIR/AppIcon.png"
swift "$ROOT/macos/render-icon.swift" "$ICON_PNG"

ICONSET="$ICON_DIR/AppIcon.iconset"
mkdir -p "$ICONSET"
sips -z 16 16 "$ICON_PNG" --out "$ICONSET/icon_16x16.png" >/dev/null
sips -z 32 32 "$ICON_PNG" --out "$ICONSET/icon_16x16@2x.png" >/dev/null
sips -z 32 32 "$ICON_PNG" --out "$ICONSET/icon_32x32.png" >/dev/null
sips -z 64 64 "$ICON_PNG" --out "$ICONSET/icon_32x32@2x.png" >/dev/null
sips -z 128 128 "$ICON_PNG" --out "$ICONSET/icon_128x128.png" >/dev/null
sips -z 256 256 "$ICON_PNG" --out "$ICONSET/icon_128x128@2x.png" >/dev/null
sips -z 256 256 "$ICON_PNG" --out "$ICONSET/icon_256x256.png" >/dev/null
sips -z 512 512 "$ICON_PNG" --out "$ICONSET/icon_256x256@2x.png" >/dev/null
sips -z 512 512 "$ICON_PNG" --out "$ICONSET/icon_512x512.png" >/dev/null
sips -z 1024 1024 "$ICON_PNG" --out "$ICONSET/icon_512x512@2x.png" >/dev/null
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICON_DIR"

echo "Signing..."
ensure_signing_identity
# --timestamp=none avoids a network wait that looks like a freeze.
if ! codesign --force --sign "$CERT_NAME" --keychain "$KEYCHAIN" --timestamp=none \
  --identifier "com.hyperheadset.menu" \
  "$APP/Contents/MacOS/HyperHeadset" \
  || ! codesign --force --sign "$CERT_NAME" --keychain "$KEYCHAIN" --timestamp=none \
  --identifier "com.hyperheadset.menu" \
  "$APP"
then
  echo "Stable signature failed. macOS may ask for device access again after each update." >&2
  codesign --force --sign - --identifier "com.hyperheadset.menu" "$APP"
fi
xattr -cr "$APP" || true

LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [[ -x "$LSREGISTER" ]]; then
  "$LSREGISTER" -f "$APP" >/dev/null || true
fi

echo "Installed $APP"
echo "Refreshing the Dock..."
killall Dock >/dev/null 2>&1 || true
echo "Open HyperHeadset from the Apps folder. Allow device access once if macOS asks. That permission is kept."
