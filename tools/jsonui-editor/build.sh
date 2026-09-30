#!/usr/bin/env bash
# Builds the static JSON-UI editor site into OUT (default target/jsonui-editor-site).
# Usage: build.sh [OUT] [MONOCRAFT_TTF]
# Needs the wasm32-unknown-unknown target and the wasm-bindgen CLI whose version
# Cargo.lock pins. Without a Monocraft source the site measures text with a fallback.
set -euo pipefail

here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
repo="$(CDPATH= cd -- "$here/../.." && pwd)"
out="${1:-$repo/target/jsonui-editor-site}"
font="${2:-}"
cargo="${CARGO:-cargo}"
case "$repo/" in
    "$out"/*) printf 'refusing to replace %s, which holds the repository\n' "$out" >&2; exit 1 ;;
esac

pinned="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$repo/Cargo.lock")"
installed="$(wasm-bindgen --version 2>/dev/null | awk '{ print $2 }' || true)"
if [[ "$installed" != "$pinned" ]]; then
    printf 'wasm-bindgen CLI %s is required (found %s): cargo install wasm-bindgen-cli --version %s --locked\n' \
        "$pinned" "${installed:-none}" "$pinned" >&2
    exit 1
fi

"$cargo" build --locked --release -p jsonui-editor --lib --target wasm32-unknown-unknown \
    --manifest-path "$repo/Cargo.toml"
target_dir="${CARGO_TARGET_DIR:-$repo/target}"

rm -rf "$out"
mkdir -p "$out/pkg" "$out/examples"
wasm-bindgen --target web --no-typescript --out-dir "$out/pkg" \
    "$target_dir/wasm32-unknown-unknown/release/jsonui_editor.wasm"
cp "$here/web/index.html" "$here/web/style.css" "$here/web/app.js" "$out/"
touch "$out/.nojekyll"

# The authored example packs, plus the listing the page fetches them by.
cp -R "$here/examples/." "$out/examples/"
(
    cd "$here/examples"
    printf '{\n  "screen": "example.example_screen",\n  "layers": {\n'
    first=1
    for layer in base overlay; do
        [[ $first == 1 ]] || printf ',\n'
        first=0
        printf '    "%s": [' "$layer"
        (cd "$layer" && find . -type f | sed 's|^\./||' | sort) | awk 'NR > 1 { printf ", " } { printf "\"%s\"", $0 }'
        printf ']'
    done
    printf '\n  }\n}\n'
) > "$out/examples/files.json"

# Monocraft (OFL-1.1), compiled by the client's own font compiler.
if [[ -n "$font" ]]; then
    scratch="$(mktemp -d)"
    trap 'rm -rf "$scratch"' EXIT
    "$cargo" run --locked --quiet -p asset-compiler --bin assetc --manifest-path "$repo/Cargo.toml" -- \
        outline-font-assets --primary-only --font "$font" \
        --source-manifest "$repo/assets/ui-font-source.json" \
        --out "$scratch/monocraft.mcbefont" --report "$scratch/report.json"
    cp "$scratch/monocraft.mcbefont" "$out/monocraft.mcbefont"
    cp "$scratch/ui-font-notices.txt" "$out/monocraft-notices.txt"
fi

bash "$here/check-site.sh" "$out"
printf 'Built %s\n' "$out"
