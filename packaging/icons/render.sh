#!/usr/bin/env bash
# render.sh <out_dir> <size>...: rasterizes cinnabar.svg to <out_dir>/icon-<size>.png.
set -euo pipefail
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
out="$1"; shift
mkdir -p "$out"
for size in "$@"; do
    if command -v rsvg-convert >/dev/null 2>&1; then
        rsvg-convert -w "$size" -h "$size" "$here/cinnabar.svg" -o "$out/icon-$size.png"
    elif command -v magick >/dev/null 2>&1; then
        magick -background none -density 384 "$here/cinnabar.svg" -resize "${size}x${size}" "$out/icon-$size.png"
    else
        echo 'install librsvg (rsvg-convert) or ImageMagick to render icons' >&2; exit 1
    fi
done
