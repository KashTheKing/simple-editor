#!/usr/bin/env bash
# Rasterise library/icons/brands/*.svg (Simple Icons, CC0) to 64 px white-on-transparent PNGs that
# src/ui/brand_icons.rs embeds. Headless Edge draws each SVG black on white, ffmpeg turns the picture
# into an alpha mask. Needs Microsoft Edge and ffmpeg on PATH. Usage: scripts/brand-icons.sh
set -euo pipefail
cd "$(dirname "$0")/../library/icons/brands"
EDGE="${EDGE:-/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
for svg in *.svg; do
    slug="${svg%.svg}"
    printf '<!doctype html><html><body style="margin:0;background:#fff;overflow:hidden">%s</body></html>' \
        "$(sed 's/<svg /<svg width="64" height="64" fill="#000" /' "$svg")" > "$tmp/$slug.html"
    # Edge can linger after writing the shot: give it 20 s, the file is there by then
    timeout 20 "$EDGE" --headless --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
        --user-data-dir="$(cygpath -w "$tmp/profile")" --window-size=64,64 \
        --screenshot="$(cygpath -w "$tmp/$slug.raw.png")" "file:///$(cygpath -m "$tmp/$slug.html")" 2>/dev/null || true
    ffmpeg -v error -y -i "$tmp/$slug.raw.png" -filter_complex \
        "[0]crop=64:64:0:0,format=gray,negate[a];color=white:s=64x64,format=rgba[w];[w][a]alphamerge,format=rgba" \
        -frames:v 1 "$slug.png"
    echo "$slug.png"
done
