#!/usr/bin/env bash
# Sourced by the platform packagers. Stages the distributable payload without any Mojang-derived
# carrier: those are built per user on first run from the bundled prep kit.
set -euo pipefail

repo_root="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"

# stage_prep_kit <kit_dir> <assetc_binary>
stage_prep_kit() {
    local kit="$1" assetc="$2" name
    rm -rf "$kit"
    mkdir -p "$kit/bin" "$kit/scripts" "$kit/assets" "$kit/data"
    install -m 0755 "$assetc" "$kit/bin/$(basename "$assetc")"
    for name in fetch-vanilla-assets.sh fetch-vanilla-assets.ps1 fetch-ui-font.sh fetch-ui-font.ps1 rename-directory-no-replace.c; do
        install -m 0644 "$repo_root/scripts/$name" "$kit/scripts/$name"
    done
    cp "$repo_root"/assets/*.json "$kit/assets/"
    cp "$repo_root"/crates/assets/data/{block-registry,block-light-registry,biome-registry}-v2168.* "$kit/data/"
    # Prebuilt so end users need no C compiler; Windows uses the PowerShell fetcher instead.
    if [[ "$(uname -s)" != MINGW* && "$(uname -s)" != MSYS* ]]; then
        cc -std=c11 -O2 "$repo_root/scripts/rename-directory-no-replace.c" -o "$kit/bin/rename-directory-no-replace"
    fi
}

# stage_resources <resource_root> <assetc_binary>: physics registry, notices, licenses, prep kit.
stage_resources() {
    local resources="$1" assetc="$2"
    mkdir -p "$resources/assets"
    install -m 0644 "$repo_root/crates/assets/data/block-physics-v2168.bin" "$resources/assets/block-physics-v2168.bin"
    install -m 0644 "$repo_root/THIRD_PARTY_NOTICES.md" "$resources/assets/THIRD_PARTY_NOTICES.md"
    mkdir -p "$resources/licenses"
    cp "$repo_root"/assets/licenses/* "$resources/licenses/"
    stage_prep_kit "$resources/prep-kit" "$assetc"
    # Optional endpoints, injected by CI; absent means the feature is off.
    [[ -z "${CINNABAR_UPDATE_URL:-}" ]] || printf '%s\n' "$CINNABAR_UPDATE_URL" > "$resources/update-url"
    [[ -z "${CINNABAR_SENTRY_DSN:-}" ]] || printf '%s\n' "$CINNABAR_SENTRY_DSN" > "$resources/sentry-dsn"
}

# require_file <path> <hint>
require_file() {
    [[ -f "$1" ]] || { printf 'missing %s (%s)\n' "$1" "$2" >&2; exit 1; }
}

# version_of: workspace package version, the single release version source.
version_of() {
    sed -n '/^\[workspace.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p;}' "$repo_root/Cargo.toml" | head -n 1
}
