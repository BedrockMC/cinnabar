#!/usr/bin/env bash
# Hermetic installer smoke tests: real checksum/extraction and mocked network/macOS tools.
set -euo pipefail
repo_root="$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)"
installer="$repo_root/packaging/install.sh"
sandbox="$(mktemp -d)"
trap 'rm -rf "$sandbox"' EXIT
mkdir -p "$sandbox/bin" "$sandbox/fixtures"
export FIXTURE_DIR="$sandbox/fixtures" TEST_LOG="$sandbox/requests.log" TEST_RUN_LOG="$sandbox/launch.log"
export PATH="$sandbox/bin:$PATH" TEST_OS=Linux TEST_ARCH=x86_64 TEST_ARM=0

cat > "$sandbox/bin/uname" <<'EOF'
#!/usr/bin/env bash
case "$1" in -s) printf '%s\n' "$TEST_OS" ;; -m) printf '%s\n' "$TEST_ARCH" ;; *) exit 1 ;; esac
EOF
cat > "$sandbox/bin/sysctl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$TEST_ARM"
EOF
cat > "$sandbox/bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
out= head=0
while [[ $# -gt 1 ]]; do
    case "$1" in
        --head) head=1; shift ;;
        --output) out="$2"; shift 2 ;;
        --proto|--tlsv1.2|--write-out) if [[ "$1" == --tlsv1.2 ]]; then shift; else shift 2; fi ;;
        *) shift ;;
    esac
done
url="$1"
printf '%s\n' "$url" >> "$TEST_LOG"
if [[ "$head" == 1 ]]; then
    printf '%s' "https://github.com/bedrock-mc/cinnabar/releases/tag/${TEST_TAG:-v9.8.7}"
else
    [[ "$url" == */download/v9.8.7/* ]] || exit 90
    cp "$FIXTURE_DIR/${url##*/}" "$out"
fi
EOF
cat > "$sandbox/bin/hdiutil" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'hdiutil %s\n' "$*" >> "$TEST_LOG"
if [[ "$1" == attach ]]; then
    while [[ "$1" != -mountpoint ]]; do shift; done
    mkdir -p "$2/Cinnabar.app/Contents"
    printf 'new app\n' > "$2/Cinnabar.app/Contents/release"
fi
EOF
cat > "$sandbox/bin/ditto" <<'EOF'
#!/usr/bin/env bash
cp -R "$1" "$2"
EOF
cat > "$sandbox/bin/xattr" <<'EOF'
#!/usr/bin/env bash
printf 'xattr %s\n' "$*" >> "$TEST_LOG"
EOF
cat > "$sandbox/bin/shasum" <<'EOF'
#!/usr/bin/env bash
[[ "$1" == -a && "$2" == 256 ]] || exit 92
sha256sum "$3"
EOF
chmod 0755 "$sandbox/bin/"*

cat > "$FIXTURE_DIR/Cinnabar-x86_64.AppImage" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == --appimage-extract ]] || exit 91
mkdir -p squashfs-root
cat > squashfs-root/AppRun <<'APP'
#!/bin/sh
printf '%s\n' "$@" > "$TEST_RUN_LOG"
APP
chmod 0755 squashfs-root/AppRun
printf 'synthetic icon\n' > squashfs-root/cinnabar.png
EOF
printf 'arm dmg\n' > "$FIXTURE_DIR/Cinnabar-arm64.dmg"
printf 'intel dmg\n' > "$FIXTURE_DIR/Cinnabar-x86_64.dmg"
checksums() {
    (cd "$FIXTURE_DIR" && sha256sum Cinnabar-*) > "$FIXTURE_DIR/SHA256SUMS.txt"
}
fresh_home() {
    export HOME="$sandbox/$1" XDG_DATA_HOME="$sandbox/$1/custom data"
    mkdir -p "$HOME"
    : > "$TEST_LOG"
}
expect_failure() {
    if bash "$installer" > "$sandbox/output" 2>&1; then
        cat "$sandbox/output" >&2
        printf 'FAIL: installer unexpectedly succeeded\n' >&2
        exit 1
    fi
    grep -F "$1" "$sandbox/output" >/dev/null
}
checksums

fresh_home "user's home"
bash "$installer" > "$sandbox/output"
[[ -x "$HOME/.local/lib/cinnabar/AppDir/AppRun" && -x "$HOME/.local/bin/cinnabar" ]]
[[ -f "$XDG_DATA_HOME/applications/cinnabar.desktop" && -f "$XDG_DATA_HOME/icons/hicolor/256x256/apps/cinnabar.png" ]]
"$HOME/.local/bin/cinnabar" 'argument with spaces' "apostrophe's argument"
printf 'argument with spaces\napostrophe\x27s argument\n' > "$sandbox/expected-arguments"
cmp "$TEST_RUN_LOG" "$sandbox/expected-arguments"
[[ "$(wc -l < "$TEST_LOG")" == 3 ]]
grep -F '/download/v9.8.7/' "$TEST_LOG" >/dev/null
[[ -z "$(find "$HOME/.local/lib/cinnabar" -maxdepth 1 -name '.install.*' -print)" ]]

printf 'old install\n' > "$HOME/.local/lib/cinnabar/AppDir/preserved"
printf 'tamper\n' >> "$FIXTURE_DIR/Cinnabar-x86_64.AppImage"
expect_failure 'SHA-256 mismatch'
[[ "$(cat "$HOME/.local/lib/cinnabar/AppDir/preserved")" == 'old install' ]]
checksums
cp "$FIXTURE_DIR/SHA256SUMS.txt" "$sandbox/checksums"
grep -v 'AppImage' "$sandbox/checksums" > "$FIXTURE_DIR/SHA256SUMS.txt"
expect_failure 'missing or ambiguous'
cat "$sandbox/checksums" "$sandbox/checksums" > "$FIXTURE_DIR/SHA256SUMS.txt"
expect_failure 'missing or ambiguous'
checksums

fresh_home 'unsupported'
export TEST_ARCH=aarch64
expect_failure 'unsupported Linux architecture'
[[ ! -s "$TEST_LOG" ]]
export TEST_ARCH=x86_64 TEST_TAG=v9.8.7-beta.1
expect_failure 'could not resolve a stable release'
[[ "$(wc -l < "$TEST_LOG")" == 1 ]]
unset TEST_TAG

for machine in arm64 x86_64 rosetta; do
    fresh_home "$machine"
    export TEST_OS=Darwin TEST_ARM=0 TEST_ARCH="$machine"
    expected_asset=Cinnabar-x86_64.dmg
    if [[ "$machine" == arm64 ]]; then expected_asset=Cinnabar-arm64.dmg; fi
    if [[ "$machine" == rosetta ]]; then
        export TEST_ARM=1 TEST_ARCH=x86_64
        expected_asset=Cinnabar-arm64.dmg
    fi
    bash "$installer" > "$sandbox/output"
    [[ "$(cat "$HOME/Applications/Cinnabar.app/Contents/release")" == 'new app' ]]
    grep -F "/download/v9.8.7/$expected_asset" "$TEST_LOG" >/dev/null
    grep -F 'xattr -dr com.apple.quarantine' "$TEST_LOG" >/dev/null
    grep -F 'hdiutil detach' "$TEST_LOG" >/dev/null
    [[ -z "$(find "$HOME/Applications" -maxdepth 1 -name '.cinnabar-install.*' -print)" ]]
done
printf 'Unix installer tests passed\n'
