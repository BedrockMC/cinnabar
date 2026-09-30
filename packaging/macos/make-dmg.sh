#!/usr/bin/env bash
# Builds a compressed DMG containing the app and an Applications link. Usage: make-dmg.sh <Cinnabar.app> <out.dmg>
set -euo pipefail
app="${1:?usage: make-dmg.sh <Cinnabar.app> <out.dmg>}"
dmg="${2:?usage: make-dmg.sh <Cinnabar.app> <out.dmg>}"
stage="$(mktemp -d)"
cp -R "$app" "$stage/Cinnabar.app"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -volname Cinnabar -srcfolder "$stage" -ov -format UDZO "$dmg"
rm -rf "$stage"
