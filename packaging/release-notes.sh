#!/usr/bin/env bash
# Prints release notes for HEAD. Usage: release-notes.sh <nightly|release> [previous_nightly_commit]
# Env: MACOS_SIGNED, WINDOWS_SIGNED ("true" when that platform's artifacts are signed).
# Markdown backticks in single-quoted prose are literal.
# shellcheck disable=SC2016
set -euo pipefail
kind="${1:?usage: release-notes.sh <nightly|release> [previous_commit]}"
previous="${2:-}"
max_commits=100

if [[ "$kind" == nightly ]]; then
    printf 'Rolling build of `main` at %s. Not a stable release; it is replaced on every push.\n\n' "$(git rev-parse --short HEAD)"
    if [[ -n "$previous" ]] && git merge-base --is-ancestor "$previous" HEAD 2>/dev/null; then
        printf '## Changes since the previous nightly (%s)\n\n' "$(git rev-parse --short "$previous")"
        range="$previous..HEAD" limit=$((max_commits + 1))
    else
        printf '## Recent changes\n\n'
        range=HEAD limit=20
    fi
    count=0
    while IFS= read -r line; do
        count=$((count + 1))
        if [[ $count -gt $max_commits ]]; then printf -- '- ...and more\n'; break; fi
        printf -- '- %s\n' "$line"
    done < <(git log --no-merges --max-count="$limit" --format='%h %s' "$range")
    [[ $count -gt 0 ]] || printf 'No new commits.\n'
    printf '\n'
fi

printf '## Installing\n\n'
printf 'Install the latest stable release with one command (scripts verify the release checksum):\n\n'
printf 'macOS and Linux:\n\n```sh\ncurl -fsSL https://github.com/bedrock-mc/cinnabar/releases/latest/download/install.sh | bash\n```\n\n'
printf 'Windows (PowerShell):\n\n```powershell\nirm https://github.com/bedrock-mc/cinnabar/releases/latest/download/install.ps1 | iex\n```\n\n'
printf 'First launch asks you to accept the Minecraft EULA, then downloads Mojang'"'"'s public sample resource pack and converts it locally; the installers themselves contain no Mojang assets.\n\n'
printf -- '- Linux: `chmod +x Cinnabar-x86_64.AppImage`, then run it.\n'
if [[ "${MACOS_SIGNED:-}" != true ]]; then
    printf -- '- macOS (unsigned build): drag Cinnabar to Applications, then run `xattr -dr com.apple.quarantine /Applications/Cinnabar.app` once (System Settings → Privacy & Security → Open Anyway also works, but setup helpers may then be blocked). Use the arm64 DMG on Apple silicon and x86_64 on Intel.\n'
else
    printf -- '- macOS: open the DMG for your Mac (arm64 for Apple silicon, x86_64 for Intel) and drag Cinnabar to Applications.\n'
fi
if [[ "${WINDOWS_SIGNED:-}" != true ]]; then
    printf -- '- Windows (unsigned build): run `Cinnabar-x64.msi`; if SmartScreen warns, choose More info → Run anyway.\n'
else
    printf -- '- Windows: run `Cinnabar-x64.msi`.\n'
fi
printf '\nVerify downloads against `SHA256SUMS.txt`.\n'
