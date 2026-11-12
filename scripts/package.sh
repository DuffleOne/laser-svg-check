#!/bin/zsh
# Builds the universal app and wraps it in a disk image, as attached to GitHub
# releases. Prints the image's path. Signs and notarises when the environment
# has what bundle.sh (SIGN_IDENTITY) and the steps below need: NOTARY_KEY_PATH
# (the .p8 file), NOTARY_KEY_ID and NOTARY_ISSUER.
set -euo pipefail
cd "${0:A:h}/.."

scripts/bundle.sh --universal >&2
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME="Laser-Check-$VERSION"
DMG="dist/$NAME.dmg"

notarise() {
  [[ -n "${NOTARY_KEY_PATH:-}" ]] || return 0
  local out
  out=$(xcrun notarytool submit "$1" \
    --key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER" --wait 2>&1) || true
  echo "$out" >&2
  [[ "$out" == *"status: Accepted"* ]] || { echo "notarisation failed for $1" >&2; exit 1; }
}

# The app first, so its own ticket is stapled and it opens offline once copied
# out of the image.
if [[ -n "${NOTARY_KEY_PATH:-}" ]]; then
  ditto -c -k --keepParent "dist/Laser Check.app" dist/notarise.zip
  notarise dist/notarise.zip
  rm dist/notarise.zip
  xcrun stapler staple "dist/Laser Check.app" >&2
fi

# The image: the app beside a link to Applications, to drag across.
rm -rf dist/dmg "$DMG" "$DMG.sha256"
mkdir -p dist/dmg
ditto "dist/Laser Check.app" "dist/dmg/Laser Check.app"
ln -s /Applications dist/dmg/Applications
hdiutil create -volname "Laser Check" -srcfolder dist/dmg -fs HFS+ -format UDZO -ov "$DMG" >&2
rm -rf dist/dmg

if [[ -n "${SIGN_IDENTITY:-}" ]]; then
  codesign --force --timestamp --sign "$SIGN_IDENTITY" "$DMG"
fi
if [[ -n "${NOTARY_KEY_PATH:-}" ]]; then
  notarise "$DMG"
  xcrun stapler staple "$DMG" >&2
fi

(cd dist && shasum -a 256 "$NAME.dmg" > "$NAME.dmg.sha256")
echo "$DMG"
