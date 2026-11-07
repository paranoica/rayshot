#!/bin/sh
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
failed=0

run() {
    monitors=$1
    cases=$2
    echo "== $monitors"
    out=$(RAYSHOT_SOAK_SUITE=displays RAYSHOT_SOAK_MONITORS="$monitors" RAYSHOT_SOAK_CASES="$cases" \
        "$HERE/soak.sh" 0 0 2>&1) || failed=1
    printf '%s\n' "$out" | grep -v '^skip' || true
}

run "1920x1080 1920x1080" plain,scale2,fractional,mixed,left,vertical,portrait,fullscreen,russian,spam,text-enter,text-escape
run "3840x2160 1920x1080" mixed,plain
run "5120x1440" plain,scale2

exit "$failed"
