#!/usr/bin/env bash
# Hermetic coverage of hdiutil retry/cleanup behavior; no macOS disk image tools required.
set -euo pipefail
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
test_root="$(mktemp -d)"
trap 'rm -rf "$test_root"' EXIT
mkdir -p "$test_root/bin" "$test_root/Cinnabar.app/Contents"
printf 'signed app bytes\n' >"$test_root/Cinnabar.app/Contents/marker"
cat >"$test_root/bin/hdiutil" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ $1 == create ]]
shift
fs='' source='' output=''
while [[ $# -gt 0 ]]; do
    case "$1" in
        -fs) fs="$2"; shift 2 ;;
        -srcfolder) source="$2"; shift 2 ;;
        -volname|-format) shift 2 ;;
        -ov) shift ;;
        *) output="$1"; shift ;;
    esac
done
[[ $fs == HFS+ && -f "$source/Cinnabar.app/Contents/marker" ]]
[[ -L "$source/Applications" && "$(readlink "$source/Applications")" == /Applications ]]
[[ "$output" != "$source/"* ]]
count=0
[[ ! -f "$CASE_ROOT/count" ]] || count="$(cat "$CASE_ROOT/count")"
count=$((count + 1))
printf '%s\n' "$count" >"$CASE_ROOT/count"
printf 'partial image\n' >"$output"
case "$MODE" in
    transient) [[ $count -lt 2 ]] || { printf 'complete image\n' >"$output"; exit 0; } ;;
    busy) ;;
    other) printf 'hdiutil: create failed - Permission denied\n' >&2; exit 19 ;;
esac
printf 'hdiutil: create failed - Resource busy\n' >&2
exit 16
STUB
cat >"$test_root/bin/sleep" <<'STUB'
#!/usr/bin/env bash
[[ $1 == 2 ]]
printf 'sleep\n' >>"$CASE_ROOT/sleeps"
STUB
chmod +x "$test_root/bin/hdiutil" "$test_root/bin/sleep"

run_case() {
    local mode="$1" expected_status="$2" expected_attempts="$3" status=0
    local case_root="$test_root/$mode"
    mkdir -p "$case_root/tmp"
    printf 'previous image\n' >"$case_root/output.dmg"
    PATH="$test_root/bin:$PATH" TMPDIR="$case_root/tmp" CASE_ROOT="$case_root" MODE="$mode" \
        bash "$here/../macos/make-dmg.sh" "$test_root/Cinnabar.app" "$case_root/output.dmg" \
        >"$case_root/log" 2>&1 || status=$?
    [[ $status -eq $expected_status ]] || { cat "$case_root/log"; exit 1; }
    [[ "$(cat "$case_root/count")" -eq $expected_attempts ]]
    [[ -z "$(find "$case_root/tmp" -mindepth 1 -print)" ]]
    [[ -z "$(find "$case_root" -name '.cinnabar-dmg.*' -print)" ]]
    if [[ $status -eq 0 ]]; then
        [[ "$(cat "$case_root/output.dmg")" == 'complete image' ]]
    else
        [[ "$(cat "$case_root/output.dmg")" == 'previous image' ]]
    fi
    if [[ $expected_attempts -eq 1 ]]; then
        [[ ! -e "$case_root/sleeps" ]]
    else
        [[ "$(wc -l <"$case_root/sleeps")" -eq $((expected_attempts - 1)) ]]
    fi
    printf 'PASS make-dmg %s (%s attempts)\n' "$mode" "$expected_attempts"
}
run_case transient 0 2
run_case busy 16 6
run_case other 19 1
