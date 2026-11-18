#!/usr/bin/env bash
# Build assets/social_icons.png, the icon atlas the device previews draw from (src/ui/social/icons.rs).
# Every name in library/icons/platforms/atlas.txt (one "<platform>/<icon> [cells wide]" per line, the order is
# the atlas order; a wordmark spans several cells) is rendered from its SVG by headless Edge at 64 px in a 72 px cell, 16 cells a row, on a
# transparent background: single-colour icons come out white (tinted at paint time), logos keep their
# colours. Needs Microsoft Edge. Usage: scripts/platform-icons.sh
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
dir="$root/library/icons/platforms"
EDGE="${EDGE:-/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
n=$(grep -c . "$dir/atlas.txt")
{
    printf '<!doctype html><html><head><style>html,body{margin:0;background:transparent;overflow:hidden}'
    printf '.c{position:absolute;width:64px;height:64px;color:#fff;fill:#fff}.c svg{width:64px;height:64px;display:block}</style></head><body>'
    col=0; row=0
    while read -r name span; do
        [ -z "$name" ] && continue
        span=${span:-1}
        if [ $((col + span)) -gt 16 ]; then col=0; row=$((row + 1)); fi
        w=$(( span * 72 - 8 ))
        printf '<div class=c style="left:%dpx;top:%dpx;width:%dpx">' $((col * 72 + 4)) $((row * 72 + 4)) "$w"
        sed -e 's/ width="[^"]*"//; s/ height="[^"]*"//' -e "s/<svg /<svg style=\"width:${w}px\" /" "$dir/$name.svg" | tr -d '\r\n'
        printf '</div>'
        col=$((col + span))
    done < "$dir/atlas.txt"
    echo $((row + 1)) > "$tmp/rows"
    printf '</body></html>'
} > "$tmp/atlas.html"
rows=$(cat "$tmp/rows")
timeout 30 "$EDGE" --headless --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --default-background-color=00000000 --user-data-dir="$(cygpath -w "$tmp/profile")" \
    --window-size=1152,$(( rows * 72 )) --screenshot="$(cygpath -w "$root/assets/social_icons.png")" \
    "file:///$(cygpath -m "$tmp/atlas.html")" 2>/dev/null || true
echo "assets/social_icons.png: $n icons, $rows rows"
