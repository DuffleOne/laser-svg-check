#!/bin/zsh
# Builds from source and installs to ~/Applications. --uninstall removes it.
set -euo pipefail
cd "${0:A:h}/.."
scripts/bundle.sh
exec scripts/install-app.sh "$@"
