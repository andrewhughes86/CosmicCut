#!/bin/sh
# Adds CosmicCut to your app menu, for your user only: the program goes in ~/.local/bin,
# the menu entry and icon in ~/.local/share. Run it from the unpacked folder.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
bin="${HOME}/.local/bin"
share="${XDG_DATA_HOME:-${HOME}/.local/share}"
mkdir -p "$bin" "$share/applications" "$share/icons/hicolor/scalable/apps"
install -m 755 "$here/CosmicCut" "$bin/cosmiccut"
install -m 644 "$here/cosmiccut.desktop" "$share/applications/cosmiccut.desktop"
install -m 644 "$here/cosmiccut.svg" "$share/icons/hicolor/scalable/apps/cosmiccut.svg"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$share/applications" || true
echo "Installed: $bin/cosmiccut (make sure $bin is on your PATH) and a CosmicCut menu entry."
