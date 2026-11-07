#!/bin/sh
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../../.." && pwd)
NAME=$1 BIN=$(realpath -e "$2") WARM=${3:-20} COLD=${4:-5} SUITE=${5:-soak} EXTRA=
case $NAME in
    fedora-39) FILE=fedora BASE=fedora:39 SHELL_MAJOR=45 EXTRA="--build-arg ARCHIVED=1" ;;
    ubuntu-24.04) FILE=debian BASE=ubuntu:24.04 SHELL_MAJOR=46 ;;
    fedora-41) FILE=fedora BASE=fedora:41 SHELL_MAJOR=47 EXTRA="--build-arg ARCHIVED=1" ;;
    debian-13) FILE=debian BASE=debian:trixie SHELL_MAJOR=48 ;;
    fedora-43) FILE=fedora BASE=fedora:43 SHELL_MAJOR=49 ;;
    ubuntu-26.04) FILE=debian BASE=ubuntu:26.04 SHELL_MAJOR=50 ;;
    arch) FILE=arch BASE= SHELL_MAJOR=51 ;;
    *) echo "unknown image $NAME" >&2; exit 2 ;;
esac
IMAGE="rayshot-soak:$NAME"
build() {
    docker build "$@" -t "$IMAGE" ${BASE:+--build-arg BASE=$BASE} $EXTRA -f "$HERE/$FILE.Dockerfile" "$HERE"
}
build -q >/dev/null || build --progress=plain
docker run --rm ${RAYSHOT_CONTAINER_CPUS:+--cpus "$RAYSHOT_CONTAINER_CPUS"} -v "$REPO:/src:ro" -v "$BIN:/opt/rayshot:ro" \
    -e RAYSHOT_SOAK_BIN=/opt/rayshot -e SHELL_RSS_LIMIT_KB=262144 \
    -e RAYSHOT_SOAK_DEBUG -e RAYSHOT_SOAK_CASES -e RAYSHOT_SOAK_MONITORS \
    "$IMAGE" sh -c '
        version=$(gnome-shell --version)
        echo "$version"
        case $version in
            "GNOME Shell $3."*) ;;
            *) echo "expected GNOME Shell $3, the image moved on" >&2; exit 1 ;;
        esac
        mkdir -p /tmp/.X11-unix /run/dbus && chmod 1777 /tmp/.X11-unix
        dbus-daemon --system --fork
        rm -rf /run/systemd/seats
        if [ "$4" = displays ] && [ -n "${RAYSHOT_SOAK_CASES:-}" ]; then
            RAYSHOT_SOAK_SUITE=displays /src/gnome-extension/tests/soak.sh 0 0 && exit 0
        elif [ "$4" = displays ]; then
            /src/gnome-extension/tests/displays.sh && exit 0
        else
            /src/gnome-extension/tests/soak.sh "$1" "$2" && exit 0
        fi
        tail -n 200 /tmp/rayshot-soak.*/shell.log; exit 1' sh "$WARM" "$COLD" "$SHELL_MAJOR" "$SUITE"
