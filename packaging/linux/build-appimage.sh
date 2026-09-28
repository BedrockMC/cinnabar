#!/usr/bin/env bash
# Builds Cinnabar-<version>-<arch>.AppImage. Requires appimagetool (APPIMAGETOOL) and librsvg or ImageMagick.
# Usage: build-appimage.sh [out_dir]. Env: CLIENT, CORE, ASSETC, ARCH (default uname -m).
set -euo pipefail
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
# shellcheck source=../common/stage-payload.sh
source "$here/../common/stage-payload.sh"

out="${1:-$repo_root/.local/dist/linux-release}"
client="${CLIENT:-$repo_root/target/release/bedrock-client}"
core="${CORE:-$repo_root/target/release/bedrock-core}"
assetc="${ASSETC:-$repo_root/target/release/assetc}"
arch="${ARCH:-$(uname -m)}"
for binary in "$client" "$core" "$assetc"; do require_file "$binary" 'build with make package-binaries'; done

appdir="$out/Cinnabar.AppDir"
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/cinnabar"
install -m 0755 "$client" "$appdir/usr/bin/bedrock-client"
install -m 0755 "$core" "$appdir/usr/bin/bedrock-core"
stage_resources "$appdir/usr/share/cinnabar" "$assetc"
install -m 0755 "$here/AppRun" "$appdir/AppRun"
install -m 0644 "$here/cinnabar.desktop" "$appdir/cinnabar.desktop"
"$here/../icons/render.sh" "$out/icons" 256
install -m 0644 "$out/icons/icon-256.png" "$appdir/cinnabar.png"

image="$out/Cinnabar-$(version_of)-$arch.AppImage"
ARCH="$arch" "${APPIMAGETOOL:-appimagetool}" "$appdir" "$image"
echo "$image"
