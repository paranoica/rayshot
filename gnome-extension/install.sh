#!/bin/sh
set -e
UUID=rayshot@paranoica
HERE=$(cd "$(dirname "$0")" && pwd)
DST="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/$UUID"
rm -rf "$DST"
mkdir -p "$DST"
cp -r "$HERE/$UUID/." "$DST/"
glib-compile-schemas "$DST/schemas"
echo "rayshot extension installed to $DST"
if gnome-extensions enable "$UUID" 2>/dev/null; then
    echo "Enabled. Wayland: log out and back in so GNOME Shell loads the new code."
else
    echo "Wayland: log out and back in, then run: gnome-extensions enable $UUID"
fi
echo "Then press Ctrl+Print to capture + open rayshot."
echo "The overlay stays warm for a faster start. To turn that off:"
echo "  gsettings --schemadir \"$DST/schemas\" set org.gnome.shell.extensions.rayshot warm-overlay false"
