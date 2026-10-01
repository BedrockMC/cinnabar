#!/usr/bin/env bash
# Install the latest stable Cinnabar release. Usage: curl -fsSL <installer-url> | bash
set -euo pipefail

fail() { printf 'Cinnabar: %s\n' "$*" >&2; exit 1; }
require() { command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"; }
require curl

platform="$(uname -s)"
architecture="$(uname -m)"
case "$platform" in
    Darwin)
        # A Rosetta terminal still needs the native Apple Silicon build.
        if [[ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" == 1 ]]; then architecture=arm64; fi
        case "$architecture" in
            arm64) asset=Cinnabar-arm64.dmg ;;
            x86_64) asset=Cinnabar-x86_64.dmg ;;
            *) fail "unsupported macOS architecture: $architecture" ;;
        esac
        for tool in shasum hdiutil ditto xattr; do require "$tool"; done
        ;;
    Linux)
        [[ "$architecture" == x86_64 ]] || fail "unsupported Linux architecture: $architecture (x86_64 required)"
        asset=Cinnabar-x86_64.AppImage
        require sha256sum
        ;;
    *) fail "unsupported operating system: $platform" ;;
esac

release_root=https://github.com/bedrock-mc/cinnabar/releases
resolved_url="$(curl --fail --silent --show-error --location --head --proto '=https' --tlsv1.2 \
    --output /dev/null --write-out '%{url_effective}' "$release_root/latest")"
tag="${resolved_url#"$release_root/tag/"}"
[[ "$resolved_url" == "$release_root/tag/$tag" && "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] \
    || fail "could not resolve a stable release from $resolved_url"
download_root="$release_root/download/$tag"

work_dir="$(mktemp -d)"
staging_dir=
mount_dir=
cleanup() {
    if [[ -n "$mount_dir" ]]; then hdiutil detach "$mount_dir" -quiet >/dev/null 2>&1 || true; fi
    [[ -z "$staging_dir" ]] || rm -rf "$staging_dir"
    rm -rf "$work_dir"
}
trap cleanup EXIT

printf 'Downloading Cinnabar %s (%s)...\n' "$tag" "$architecture"
for name in SHA256SUMS.txt "$asset"; do
    curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
        --output "$work_dir/$name" "$download_root/$name"
done
expected="$(awk -v name="$asset" '$2 == name || $2 == "*" name { print $1 }' "$work_dir/SHA256SUMS.txt")"
[[ "$expected" =~ ^[[:xdigit:]]{64}$ ]] || fail "missing or ambiguous SHA-256 checksum for $asset"
if [[ "$platform" == Darwin ]]; then
    actual="$(shasum -a 256 "$work_dir/$asset")"
else
    actual="$(sha256sum "$work_dir/$asset")"
fi
actual="${actual%% *}"
[[ "$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')" == \
   "$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')" ]] || fail "SHA-256 mismatch for $asset"

if [[ "$platform" == Darwin ]]; then
    applications="$HOME/Applications"
    mkdir -p "$applications"
    staging_dir="$(mktemp -d "$applications/.cinnabar-install.XXXXXX")"
    mount_dir="$work_dir/mount"
    mkdir -p "$mount_dir"
    hdiutil attach "$work_dir/$asset" -mountpoint "$mount_dir" -nobrowse -readonly -quiet
    [[ -d "$mount_dir/Cinnabar.app" ]] || fail "release image does not contain Cinnabar.app"
    ditto "$mount_dir/Cinnabar.app" "$staging_dir/Cinnabar.app"
    # Release builds may be ad-hoc signed; helpers must be allowed to run too.
    xattr -dr com.apple.quarantine "$staging_dir/Cinnabar.app"
    rm -rf "$applications/Cinnabar.app"
    mv "$staging_dir/Cinnabar.app" "$applications/Cinnabar.app"
    printf 'Installed %s to %s/Cinnabar.app. Open it to complete first-run setup.\n' "$tag" "$applications"
else
    install_root="$HOME/.local/lib/cinnabar"
    bin_dir="$HOME/.local/bin"
    data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
    mkdir -p "$install_root" "$bin_dir" "$data_dir/applications" "$data_dir/icons/hicolor/256x256/apps"
    staging_dir="$(mktemp -d "$install_root/.install.XXXXXX")"
    chmod 0755 "$work_dir/$asset"
    # Extract once so launching needs neither FUSE nor extraction on every run.
    (cd "$staging_dir" && "$work_dir/$asset" --appimage-extract >/dev/null)
    [[ -x "$staging_dir/squashfs-root/AppRun" ]] || fail "release image does not contain an executable AppRun"
    [[ -f "$staging_dir/squashfs-root/cinnabar.png" ]] || fail "release image does not contain its application icon"
    install -m 0644 "$staging_dir/squashfs-root/cinnabar.png" "$data_dir/icons/hicolor/256x256/apps/cinnabar.png"
    rm -rf "$install_root/AppDir"
    mv "$staging_dir/squashfs-root" "$install_root/AppDir"
    # Single-quote paths safely, including homes containing apostrophes.
    launcher_path="$(printf '%s' "$install_root/AppDir/AppRun" | sed "s/'/'\\\"'\\\"'/g")"
    printf '#!/bin/sh\nexec '\''%s'\'' "$@"\n' "$launcher_path" > "$bin_dir/cinnabar"
    chmod 0755 "$bin_dir/cinnabar"
    # Desktop Exec uses its own escaping rules, followed by command-line quoting.
    # shellcheck disable=SC2016
    desktop_path="$(printf '%s' "$bin_dir/cinnabar" | sed 's/\\/\\\\\\\\/g; s/"/\\\\"/g; s/`/\\\\`/g; s/\$/\\\\$/g; s/%/%%/g')"
    cat > "$data_dir/applications/cinnabar.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Cinnabar
Comment=Minecraft Bedrock client
Exec="$desktop_path" %U
Icon=cinnabar
Categories=Game;
Terminal=false
EOF
    if command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database "$data_dir/applications" >/dev/null 2>&1 || true; fi
    printf 'Installed %s. Launch Cinnabar from your applications menu or %s/cinnabar.\n' "$tag" "$bin_dir"
    case ":${PATH:-}:" in *":$bin_dir:"*) ;; *) printf 'For terminal use, add %s to PATH.\n' "$bin_dir" ;; esac
fi
