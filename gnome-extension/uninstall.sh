#!/bin/sh
UUID=rayshot@paranoica
DST="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/$UUID"
gnome-extensions disable "$UUID" 2>/dev/null || true
# install.sh pinned the binary path; a packaged copy should find rayshot on PATH.
if [ -d "$DST/schemas" ] && ! gsettings --schemadir "$DST/schemas" reset org.gnome.shell.extensions.rayshot binary 2>/dev/null; then
    echo "Could not reset the rayshot binary path. From your desktop session, run:"
    echo "  dconf reset /org/gnome/shell/extensions/rayshot/binary"
fi
rm -rf "$DST"
echo "Log out and back in to fully unload it from GNOME Shell."
