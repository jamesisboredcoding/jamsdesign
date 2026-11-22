#!/bin/sh
# Installs Gallerion's launcher and icon for the current user, so KDE (and other desktops) show
# the logo in the taskbar, task switcher and app menu. Usage, from the gallerion folder:
#   packaging/install-desktop.sh [path/to/gallerion-binary]   (default: ../target/release/gallerion, the suite workspace)
# Undo: rm ~/.local/share/applications/dev.jamsdesign.Gallerion.desktop \
#          ~/.local/share/icons/hicolor/scalable/apps/dev.jamsdesign.Gallerion.svg
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
bin=$(realpath "${1:-$here/../target/release/gallerion}")
data=${XDG_DATA_HOME:-$HOME/.local/share}
mkdir -p "$data/applications" "$data/icons/hicolor/scalable/apps"
cp "$here/../img/gallerion.svg" "$data/icons/hicolor/scalable/apps/dev.jamsdesign.Gallerion.svg"
sed "s|^Exec=gallerion|Exec=$bin|" "$here/packaging/dev.jamsdesign.Gallerion.desktop" \
    > "$data/applications/dev.jamsdesign.Gallerion.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true
echo "Installed: $data/applications/dev.jamsdesign.Gallerion.desktop (Exec=$bin)"
