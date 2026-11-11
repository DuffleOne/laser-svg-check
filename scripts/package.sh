#!/bin/zsh
# Builds the universal app and zips it up with the installer, as attached to
# GitHub releases. Prints the zip's path. Signs and notarises when the
# environment has what bundle.sh and the notarise step below need.
set -euo pipefail
cd "${0:A:h}/.."

scripts/bundle.sh --universal >&2
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

# Notarise when an App Store Connect API key is available: NOTARY_KEY_PATH
# (the .p8 file), NOTARY_KEY_ID and NOTARY_ISSUER. Stapling the ticket lets
# the app open without a network check.
if [[ -n "${NOTARY_KEY_PATH:-}" ]]; then
  ditto -c -k --keepParent "dist/Laser Check.app" dist/notarise.zip
  xcrun notarytool submit dist/notarise.zip \
    --key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER" --wait >&2
  xcrun stapler staple "dist/Laser Check.app" >&2
  rm dist/notarise.zip
fi
NAME="Laser-Check-$VERSION-macOS"
rm -rf "dist/pkg" "dist/$NAME.zip" "dist/$NAME.zip.sha256"
mkdir -p "dist/pkg/Laser Check"
ditto "dist/Laser Check.app" "dist/pkg/Laser Check/Laser Check.app"
cp scripts/install-app.sh "dist/pkg/Laser Check/Install Laser Check.command"
chmod +x "dist/pkg/Laser Check/Install Laser Check.command"
(cd dist/pkg && ditto -c -k --keepParent "Laser Check" "../$NAME.zip")
(cd dist && shasum -a 256 "$NAME.zip" > "$NAME.zip.sha256")
echo "dist/$NAME.zip"
