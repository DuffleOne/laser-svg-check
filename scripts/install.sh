#!/bin/zsh
# Builds from source and installs. Same flags as install-app.sh.
set -euo pipefail
cd "${0:A:h}/.."
scripts/bundle.sh
exec scripts/install-app.sh "$@"
