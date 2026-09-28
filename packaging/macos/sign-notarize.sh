#!/usr/bin/env bash
# Signs and notarizes a .app or .dmg. Usage: sign-notarize.sh <path/to/Cinnabar.app|Cinnabar.dmg>
# Env (never committed): CODESIGN_IDENTITY, and either NOTARY_PROFILE (notarytool keychain profile)
# or APPLE_ID + APPLE_TEAM_ID + APPLE_APP_PASSWORD.
set -euo pipefail
target="${1:?usage: sign-notarize.sh <app-or-dmg>}"
: "${CODESIGN_IDENTITY:?set CODESIGN_IDENTITY to a Developer ID Application identity}"

notary_args=()
if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    notary_args=(--keychain-profile "$NOTARY_PROFILE")
else
    : "${APPLE_ID:?set NOTARY_PROFILE or APPLE_ID/APPLE_TEAM_ID/APPLE_APP_PASSWORD}"
    : "${APPLE_TEAM_ID:?}" "${APPLE_APP_PASSWORD:?}"
    notary_args=(--apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD")
fi

sign() { codesign --force --timestamp --options runtime --sign "$CODESIGN_IDENTITY" "$@"; }

case "$target" in
    *.app)
        # Nested code first (helpers under Resources, then the bundle's own executables), bundle last.
        while IFS= read -r -d '' file; do
            if file "$file" | grep -q 'Mach-O'; then sign "$file"; fi
        done < <(find "$target/Contents/Resources" -type f -perm -u+x -print0)
        sign "$target/Contents/MacOS/bedrock-core"
        sign "$target"
        codesign --verify --deep --strict --verbose=2 "$target"
        archive="$(mktemp -d)/Cinnabar.zip"
        ditto -c -k --keepParent "$target" "$archive"
        xcrun notarytool submit "$archive" "${notary_args[@]}" --wait
        xcrun stapler staple "$target"
        spctl --assess --type execute --verbose "$target"
        ;;
    *.dmg)
        sign "$target"
        xcrun notarytool submit "$target" "${notary_args[@]}" --wait
        xcrun stapler staple "$target"
        spctl --assess --type open --context context:primary-signature --verbose "$target"
        ;;
    *) echo "unsupported target: $target" >&2; exit 2 ;;
esac
