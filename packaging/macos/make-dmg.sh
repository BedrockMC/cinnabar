#!/usr/bin/env bash
# Builds a compressed DMG containing the app and an Applications link. Usage: make-dmg.sh <Cinnabar.app> <out.dmg>
set -euo pipefail
app="${1:?usage: make-dmg.sh <Cinnabar.app> <out.dmg>}"
dmg="${2:?usage: make-dmg.sh <Cinnabar.app> <out.dmg>}"
mkdir -p "$(dirname -- "$dmg")"
output_dir="$(CDPATH= cd -- "$(dirname -- "$dmg")" && pwd)"
work="$(mktemp -d "$output_dir/.cinnabar-dmg.XXXXXX")"
trap 'rm -rf "$work"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir "$work/payload"
cp -R "$app" "$work/payload/Cinnabar.app"
ln -s /Applications "$work/payload/Applications"

# Select HFS+ explicitly rather than inheriting the host's default filesystem. Intel CI
# has also returned transient "Resource busy" failures; retry that error alone, keeping
# partial images outside the source folder and preserving the previous output on failure.
for attempt in {1..6}; do
    rm -f "$work/output.dmg"
    if hdiutil create -volname Cinnabar -fs HFS+ -srcfolder "$work/payload" \
        -ov -format UDZO "$work/output.dmg" >"$work/create.log" 2>&1; then
        cat "$work/create.log"
        # The work folder shares the output filesystem, so publishing is a rename.
        mv -f "$work/output.dmg" "$dmg"
        exit 0
    else
        status=$?
    fi
    cat "$work/create.log" >&2
    if [[ $attempt -eq 6 ]] || ! grep -qF 'Resource busy' "$work/create.log"; then
        exit "$status"
    fi
    printf 'Retrying DMG creation after Resource busy (%s/6)\n' "$attempt" >&2
    sleep 2
done
