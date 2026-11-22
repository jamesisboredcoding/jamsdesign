#!/bin/sh
# Installs Photosplash's launcher and icon for the current user, so KDE (and other desktops) show
# the logo in the taskbar, task switcher and app menu. Usage, from the photosplash folder:
#   packaging/install-desktop.sh [path/to/photosplash-binary]   (default: ../target/release/photosplash, the suite workspace)
# Undo: rm ~/.local/share/applications/dev.jamsdesign.Photosplash.desktop \
#          ~/.local/share/icons/hicolor/scalable/apps/dev.jamsdesign.Photosplash.svg
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
bin=$(realpath "${1:-$here/../target/release/photosplash}")
data=${XDG_DATA_HOME:-$HOME/.local/share}
mkdir -p "$data/applications" "$data/icons/hicolor/scalable/apps"
cp "$here/../img/photosplash.svg" "$data/icons/hicolor/scalable/apps/dev.jamsdesign.Photosplash.svg"
sed "s|^Exec=photosplash|Exec=$bin|" "$here/packaging/dev.jamsdesign.Photosplash.desktop" \
    > "$data/applications/dev.jamsdesign.Photosplash.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true
echo "Installed: $data/applications/dev.jamsdesign.Photosplash.desktop (Exec=$bin)"
