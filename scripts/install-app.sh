#!/bin/zsh
# Installs dist/Laser Check.app (built by bundle.sh) into ~/Applications and
# starts the folder watcher at login. Releases ship a disk image instead,
# where opening the app sets up the watcher itself.
#
#   scripts/install-app.sh               install, start the watcher
#   scripts/install-app.sh --uninstall   remove the watcher and the app
set -euo pipefail

SRC="${0:A:h}/../dist/Laser Check.app"
APP="$HOME/Applications/Laser Check.app"

if [[ "${1:-}" == "--uninstall" ]]; then
  [[ -x "$APP/Contents/MacOS/laser-check" ]] && "$APP/Contents/MacOS/laser-check" uninstall >/dev/null
  rm -rf "$APP"
  echo "removed the watcher and $APP"
  echo "settings and history are left in ~/.config/laser-check and ~/Library/Application Support/laser-check"
  exit 0
fi

[[ -d "$SRC" ]] || { echo "no $SRC; run scripts/bundle.sh first" >&2; exit 1; }

mkdir -p "$HOME/Applications"
# Replace by rename so an open copy of the app doesn't crash mid-update.
rm -rf "$APP.new"
ditto "$SRC" "$APP.new"
rm -rf "$APP"
mv "$APP.new" "$APP"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP" || true

"$APP/Contents/MacOS/laser-check" install-agent
# Restart it even if the agent file didn't change, so it runs this build.
launchctl kickstart -k "gui/$(id -u)/one.duffle.laser-check"
echo "installed $APP, watcher running (log: ~/Library/Logs/laser-check.log)"
