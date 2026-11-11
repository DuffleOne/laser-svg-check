#!/bin/zsh
# Builds the universal app and zips it up with the installer, as attached to
# GitHub releases. Prints the zip's path.
set -euo pipefail
cd "${0:A:h}/.."

scripts/bundle.sh --universal >&2
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME="Laser-Check-$VERSION-macOS"
rm -rf "dist/pkg" "dist/$NAME.zip" "dist/$NAME.zip.sha256"
mkdir -p "dist/pkg/Laser Check"
ditto "dist/Laser Check.app" "dist/pkg/Laser Check/Laser Check.app"
cp scripts/install-app.sh "dist/pkg/Laser Check/Install Laser Check.command"
chmod +x "dist/pkg/Laser Check/Install Laser Check.command"
(cd dist/pkg && ditto -c -k --keepParent "Laser Check" "../$NAME.zip")
(cd dist && shasum -a 256 "$NAME.zip" > "$NAME.zip.sha256")
echo "dist/$NAME.zip"
