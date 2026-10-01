#!/usr/bin/env bash
# Fails when an extracted installer holds any file outside the allowlist, so no Mojang asset (or
# anything else unreviewed) can ship. Missing expected files fail too.
# Usage: check-payload.sh <macos|linux|windows> <mounted DMG | squashfs-root | MSI admin image>
set -euo pipefail
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
# shellcheck source=common/stage-payload.sh
source "$here/common/stage-payload.sh"

platform="${1:?usage: check-payload.sh <macos|linux|windows> <dir>}"
root="$(CDPATH= cd -- "${2:?usage: check-payload.sh <macos|linux|windows> <dir>}" && pwd)"

# Required and optional entries relative to $root; optional ones may be absent.
case "$platform" in
    macos)
        prefix=Cinnabar.app/Contents/
        resources="${prefix}Resources/"
        required=(Applications "${prefix}Info.plist" "${resources}Cinnabar.icns")
        for name in bedrock-client bedrock-core bedrock-local-server; do required+=("${prefix}MacOS/$name"); done
        # CodeResources at the top of Contents is the stapled notarization ticket.
        optional=("${prefix}_CodeSignature/CodeResources" "${prefix}CodeResources")
        ;;
    linux)
        prefix=''
        resources=usr/share/cinnabar/
        required=(AppRun cinnabar.desktop cinnabar.png)
        for name in bedrock-client bedrock-core bedrock-local-server; do required+=("usr/bin/$name"); done
        optional=(.DirIcon)
        ;;
    windows)
        # The admin image nests the install dir under a standard-folder name; find it by the client.
        client="$(cd "$root" && find . -type f -name bedrock-client.exe | sed 's|^\./||')"
        [[ -n "$client" && "$client" != *$'\n'* ]] || { echo "expected exactly one bedrock-client.exe under $root" >&2; exit 1; }
        prefix="$(dirname -- "$client")/"
        [[ "$prefix" != ./ ]] || prefix=''
        resources="${prefix}resources/"
        required=()
        for name in bedrock-client bedrock-core bedrock-local-server; do required+=("$prefix$name.exe"); done
        optional=()
        # msiexec /a leaves a copy of the package beside the image.
        while IFS= read -r msi; do optional+=("$msi"); done < <(cd "$root" && find . -maxdepth 1 -type f -name '*.msi' | sed 's|^\./||')
        ;;
    *) echo "unknown platform: $platform" >&2; exit 2 ;;
esac
while IFS= read -r file; do required+=("$resources$file"); done < <(resource_manifest "$platform")
optional+=("${resources}update-url")

allowed="$(printf '%s\n' "${required[@]}" "${optional[@]}" | sort -u)"
actual="$(cd "$root" && find . ! -type d | sed 's|^\./||' | sort)"
status=0
while IFS= read -r file; do
    [[ -z "$file" ]] && continue
    if ! grep -qxF -- "$file" <<<"$allowed"; then
        printf 'unexpected file in %s artifact: %s\n' "$platform" "$file" >&2
        status=1
    fi
done <<<"$actual"
for file in "${required[@]}"; do
    if ! grep -qxF -- "$file" <<<"$actual"; then
        printf 'missing file in %s artifact: %s\n' "$platform" "$file" >&2
        status=1
    fi
done
[[ $status -ne 0 ]] || printf '%s artifact holds only allowlisted files (%s entries)\n' "$platform" "$(wc -l <<<"$actual" | tr -d ' ')"
exit $status
