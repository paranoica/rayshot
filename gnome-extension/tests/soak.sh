#!/bin/sh
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/rayshot-soak.XXXXXX")
SERVER_RSS_LIMIT_KB=${SERVER_RSS_LIMIT_KB:-16384}
SERVER_VRAM_LIMIT_MIB=${SERVER_VRAM_LIMIT_MIB:-16}
SHELL_RSS_LIMIT_KB=${SHELL_RSS_LIMIT_KB:-16384}
TIMEOUT_S=${TIMEOUT_S:-600}

TAG="rayshot-soak-$$-$(date +%s%N)"
SHELL_PID=
KEEP_WORK=${SOAK_KEEP:-}

descendants() {
    for child in $(pgrep -P "$1"); do
        descendants "$child"
        echo "$child"
    done
}

tagged() {
    for pid in $(pgrep -u "$(id -u)"); do
        if tr '\0' '\n' 2>/dev/null <"/proc/$pid/environ" | grep -qx "RAYSHOT_NESTED_TAG=$TAG"; then
            echo "$pid"
        fi
    done
    return 0
}

cleanup() {
    set +e
    pids="$(tagged)"
    [ -n "$SHELL_PID" ] && pids="$pids $(descendants "$SHELL_PID") $SHELL_PID"
    if [ -n "$(echo $pids)" ]; then
        kill $pids 2>/dev/null || true
        sleep 1
        kill -9 $pids 2>/dev/null || true
    fi
    for mnt in $(awk -v w="$WORK" 'index($2, w) == 1 {print $2}' /proc/mounts); do
        fusermount3 -uz "$mnt" 2>/dev/null || fusermount -uz "$mnt" 2>/dev/null || true
    done
    [ -z "$KEEP_WORK" ] && rm -rf "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT HUP TERM

BIN="${RAYSHOT_SOAK_BIN:-$REPO/target/release/rayshot}"
if [ ! -x "$BIN" ]; then
    echo "soak: build first, $BIN is missing" >&2
    exit 1
fi
if [ -z "${RAYSHOT_SOAK_BIN:-}" ] && [ -n "$(find "$REPO/src" -newer "$BIN" -name '*.rs' | head -n1)" ]; then
    echo "soak: $BIN is older than src/, rebuild it" >&2
    exit 1
fi

export XDG_DATA_HOME="$WORK/data" XDG_CONFIG_HOME="$WORK/config"
export XDG_CACHE_HOME="$WORK/cache" XDG_STATE_HOME="$WORK/state"
export XDG_RUNTIME_DIR="$WORK/runtime"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME"
chmod 700 "$XDG_RUNTIME_DIR"

EXT="$XDG_DATA_HOME/gnome-shell/extensions"
mkdir -p "$EXT"
cp -r "$REPO/gnome-extension/rayshot@paranoica" "$EXT/"
cp -r "$HERE/soak/rayshot-soak@paranoica" "$EXT/"
glib-compile-schemas "$EXT/rayshot@paranoica/schemas"

mkdir -p "$WORK/bin"
REAL_WL_COPY=$(command -v wl-copy) || { echo "soak: wl-copy is not installed" >&2; exit 1; }
cat >"$WORK/bin/wl-copy" <<WLCOPY
#!/bin/sh
if [ -e "$WORK/wl-copy-hang" ]; then cat >/dev/null; exec sleep 30; fi
exec "$REAL_WL_COPY" "\$@"
WLCOPY
chmod +x "$WORK/bin/wl-copy"
export PATH="$WORK/bin:$PATH" RAYSHOT_SOAK_WLCOPY_HANG="$WORK/wl-copy-hang"

ANY_SHELL=false
[ -n "${RAYSHOT_SOAK_ANY_SHELL:-}" ] && ANY_SHELL=true

export RAYSHOT_BIN="$BIN"
export RAYSHOT_SOAK_OUT="$WORK/result.json"
export RAYSHOT_SOAK_WARM="${1:-100}" RAYSHOT_SOAK_COLD="${2:-20}"
export RAYSHOT_SOAK_SUITE="${RAYSHOT_SOAK_SUITE:-full}"
MONITORS=""
for size in ${RAYSHOT_SOAK_MONITORS:-1920x1080 1920x1080}; do
    MONITORS="$MONITORS --virtual-monitor $size"
done
MUTTER_FEATURES="[]"
if [ "$RAYSHOT_SOAK_SUITE" = displays ]; then
    MUTTER_FEATURES="['scale-monitor-framebuffer']"
    export RAYSHOT_NO_REMEMBER_SELECTION=1
fi
case "${RAYSHOT_SOAK_DEBUG:-}" in
    ""|0|false|no) DEBUG_LOG=false ;;
    *) DEBUG_LOG=true ;;
esac
unset WAYLAND_DISPLAY DISPLAY

RAYSHOT_NESTED_TAG="$TAG" dbus-run-session -- sh -c "
    gsettings set org.gnome.shell disable-user-extensions false
    gsettings set org.gnome.shell enabled-extensions \"['rayshot@paranoica', 'rayshot-soak@paranoica']\"
    gsettings set org.gnome.shell welcome-dialog-last-shown-version '999'
    gsettings set org.gnome.shell disable-extension-version-validation $ANY_SHELL
    gsettings set org.gnome.mutter experimental-features \"$MUTTER_FEATURES\"
    gsettings --schemadir \"$EXT/rayshot@paranoica/schemas\" set org.gnome.shell.extensions.rayshot debug-log $DEBUG_LOG
    gsettings set org.gnome.desktop.input-sources sources \"[('xkb', 'us'), ('xkb', 'ru')]\"
    exec gnome-shell --headless --wayland $MONITORS
" >"$WORK/shell.log" 2>&1 &
SHELL_PID=$!

waited=0
while [ ! -s "$RAYSHOT_SOAK_OUT" ]; do
    if ! kill -0 "$SHELL_PID" 2>/dev/null; then
        echo "soak: nested shell died, log: $WORK/shell.log" >&2
        KEEP_WORK=1
        exit 1
    fi
    if [ "$waited" -ge "$TIMEOUT_S" ]; then
        echo "soak: timed out after ${TIMEOUT_S}s, log: $WORK/shell.log" >&2
        KEEP_WORK=1
        exit 1
    fi
    sleep 1
    waited=$((waited + 1))
done

gjs -m "$HERE/soak/verdict.js" "$RAYSHOT_SOAK_OUT" \
    "$SERVER_RSS_LIMIT_KB" "$SERVER_VRAM_LIMIT_MIB" "$SHELL_RSS_LIMIT_KB" || {
    echo "soak: log kept in $WORK" >&2
    KEEP_WORK=1
    exit 1
}
