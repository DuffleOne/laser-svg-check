#!/bin/zsh
# Installs a built Laser Check.app into ~/Applications and starts the folder
# watcher at login. Ships inside the release zip as install.sh, so it must not
# need Rust or the source tree.
#
#   ./install.sh                 app + watcher
#   ./install.sh --no-agent      app only
#   ./install.sh --uninstall     remove the watcher and the app
set -euo pipefail

HERE="${0:A:h}"
SRC="$HERE/Laser Check.app"
[[ -d "$SRC" ]] || SRC="$HERE/../dist/Laser Check.app"
APP="$HOME/Applications/Laser Check.app"
LABEL="one.duffle.laser-check"
AGENT="$HOME/Library/LaunchAgents/$LABEL.plist"
DOMAIN="gui/$(id -u)"

if [[ "${1:-}" == "--uninstall" ]]; then
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  rm -f "$AGENT"
  rm -rf "$APP"
  echo "removed the watcher and $APP"
  echo "settings and history are left in ~/.config/laser-check and ~/Library/Application Support/laser-check"
  exit 0
fi

[[ -d "$SRC" ]] || { echo "can't find Laser Check.app next to this script" >&2; exit 1; }

mkdir -p "$HOME/Applications" "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
# Replace by rename so an open copy of the app doesn't crash mid-update.
rm -rf "$APP.new"
ditto "$SRC" "$APP.new"
xattr -dr com.apple.quarantine "$APP.new" 2>/dev/null || true
rm -rf "$APP"
mv "$APP.new" "$APP"
codesign --force --sign - "$APP" >/dev/null 2>&1 || true
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP" || true

if [[ "${1:-}" == "--no-agent" ]]; then
  echo "installed $APP (watcher not touched)"
  exit 0
fi

cat > "$AGENT" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array><string>$APP/Contents/MacOS/laser-check</string><string>watch</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Background</string>
  <key>LowPriorityIO</key><true/>
  <key>StandardErrorPath</key><string>$HOME/Library/Logs/laser-check.log</string>
</dict>
</plist>
EOF
launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
launchctl bootstrap "$DOMAIN" "$AGENT"
echo "installed $APP, watcher running (log: ~/Library/Logs/laser-check.log)"
