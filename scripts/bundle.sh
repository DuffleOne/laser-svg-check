#!/bin/zsh
# Builds dist/Laser Check.app.
#
#   scripts/bundle.sh              this Mac's architecture
#   scripts/bundle.sh --universal  Apple silicon + Intel in one binary (what releases ship)
#
# Set SIGN_IDENTITY (e.g. "Developer ID Application") to sign for distribution;
# otherwise the app is signed ad hoc.
set -euo pipefail
cd "${0:A:h}/.."

export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP="dist/Laser Check.app"

if [[ "${1:-}" == "--universal" ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p target/universal
  lipo -create -output target/universal/laser-check \
    target/aarch64-apple-darwin/release/laser-check \
    target/x86_64-apple-darwin/release/laser-check
  BIN=target/universal/laser-check
else
  cargo build --release
  BIN=target/release/laser-check
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/laser-check"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
sed "s/__VERSION__/$VERSION/g" scripts/Info.plist > "$APP/Contents/Info.plist"
if [[ -n "${SIGN_IDENTITY:-}" ]]; then
  # Developer ID with the hardened runtime and a timestamp, as notarisation needs.
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP"
else
  # Ad-hoc: enough for this Mac (and Apple silicon) to run it.
  codesign --force --sign - "$APP"
fi
echo "built $APP ($VERSION)"
