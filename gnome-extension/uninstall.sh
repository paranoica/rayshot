#!/bin/sh
UUID=rayshot@paranoica
gnome-extensions disable "$UUID" 2>/dev/null || true
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/$UUID"
echo "Log out and back in to fully unload it from GNOME Shell."
