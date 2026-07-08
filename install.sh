#!/usr/bin/env bash
# Install Workday Timer for the current user: builds the release binary,
# copies it to ~/.local/bin, and adds a desktop launcher.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"

echo "==> Building release binary..."
cargo build --release

bindir="$HOME/.local/bin"
appdir="$HOME/.local/share/applications"
mkdir -p "$bindir" "$appdir"

echo "==> Installing binary to $bindir/workday-timer"
install -m 0755 target/release/workday_timer "$bindir/workday-timer"

echo "==> Installing desktop launcher"
cat > "$appdir/workday-timer.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Workday Timer
Comment=Menu-bar countdown of your remaining 8-hour workday
Exec=$bindir/workday-timer
Icon=alarm-symbolic
Terminal=false
Categories=Utility;
StartupNotify=false
StartupWMClass=workday_timer
EOF

echo
echo "Done. Launch 'Workday Timer' from your app grid, or run: $bindir/workday-timer"
echo "To start it automatically at login:"
echo "    cp \"$appdir/workday-timer.desktop\" \"$HOME/.config/autostart/\""
