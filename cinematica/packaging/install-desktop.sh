#!/bin/sh
# Installs Cinematica's launcher and icon for the current user, so KDE (and other desktops) show
# the logo in the taskbar, task switcher and app menu. Usage, from the cinematica folder:
#   packaging/install-desktop.sh [path/to/cinematica-binary]   (default: ../target/release/cinematica)
# Undo: rm ~/.local/share/applications/dev.jamsdesign.Cinematica.desktop \
#          ~/.local/share/icons/hicolor/scalable/apps/dev.jamsdesign.Cinematica.svg
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
bin=$(realpath "${1:-$here/../target/release/cinematica}")
data=${XDG_DATA_HOME:-$HOME/.local/share}
mkdir -p "$data/applications" "$data/icons/hicolor/scalable/apps"
cp "$here/../img/cinematica.svg" "$data/icons/hicolor/scalable/apps/dev.jamsdesign.Cinematica.svg"
sed "s|^Exec=cinematica|Exec=$bin|" "$here/packaging/dev.jamsdesign.Cinematica.desktop" \
    > "$data/applications/dev.jamsdesign.Cinematica.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true
echo "Installed: $data/applications/dev.jamsdesign.Cinematica.desktop (Exec=$bin)"
