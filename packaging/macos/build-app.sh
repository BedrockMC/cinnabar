#!/usr/bin/env bash
# Assembles an unsigned Cinnabar.app from release binaries. Usage: build-app.sh [out_dir]
# Env: CLIENT, CORE, ASSETC (default target/release/*), BUNDLE_ID, CINNABAR_UPDATE_URL, CINNABAR_SENTRY_DSN.
set -euo pipefail
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
# shellcheck source=../common/stage-payload.sh
source "$here/../common/stage-payload.sh"

out="${1:-$repo_root/.local/dist/macos-release}"
client="${CLIENT:-$repo_root/target/release/bedrock-client}"
core="${CORE:-$repo_root/target/release/bedrock-core}"
assetc="${ASSETC:-$repo_root/target/release/assetc}"
bundle_id="${BUNDLE_ID:-app.cinnabar.client}"
for binary in "$client" "$core" "$assetc"; do require_file "$binary" 'build with make package-binaries'; done

app="$out/Cinnabar.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 0755 "$client" "$app/Contents/MacOS/bedrock-client"
install -m 0755 "$core" "$app/Contents/MacOS/bedrock-core"
stage_resources "$app/Contents/Resources" "$assetc"

sed -e "s|@BUNDLE_ID@|$bundle_id|g" -e "s|@VERSION@|$(version_of)|g" "$here/Info.plist.in" > "$app/Contents/Info.plist"

iconset="$(mktemp -d)/Cinnabar.iconset"
"$here/../icons/render.sh" "$iconset.png" 16 32 64 128 256 512 1024
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    cp "$iconset.png/icon-$size.png" "$iconset/icon_${size}x${size}.png"
    cp "$iconset.png/icon-$((size * 2)).png" "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Cinnabar.icns"
plutil -lint "$app/Contents/Info.plist"
echo "$app"
