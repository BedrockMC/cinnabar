"""Run the Linux installer offline against release/download/runtime fixtures."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
CONFIG = json.loads((ROOT / "packaging/release-assets.json").read_text())
ASSET = CONFIG["assets"]["linux"]["x86_64"]
BASE = f'https://github.com/{CONFIG["repository"]}/releases'

CURL = '''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import shutil
import sys
args = sys.argv[1:]
root = Path(os.environ['FIXTURE_ROOT'])
with (root / 'requests').open('a') as log:
    log.write(json.dumps(args) + '\\n')
base = os.environ['FIXTURE_BASE']
tag = os.environ['FIXTURE_TAG']
url = args[-1]
if url == base + '/latest':
    if os.environ.get('FIXTURE_NO_RELEASE'):
        sys.exit(22)
    print(base + '/tag/' + tag, end='')
    sys.exit(0)
prefix = base + '/download/' + tag + '/'
if not url.startswith(prefix):
    sys.exit(22)
asset = root / 'release' / url[len(prefix):]
if not asset.is_file() or asset.name == os.environ.get('FIXTURE_FAIL_ASSET'):
    sys.exit(22)
shutil.copyfile(asset, args[args.index('--output') + 1])
'''

APPIMAGE = '''#!/bin/sh
if [ "$1" = --appimage-extract ]; then
    printf 'extract\\n' >> "$FIXTURE_ROOT/runtime"
    [ "${FIXTURE_EXTRACT_FAIL:-}" != 1 ] || exit 17
    mkdir -p squashfs-root
    printf 'icon' > squashfs-root/cinnabar.png
else
    printf '%s\\n' "$@" > "$FIXTURE_ROOT/launch"
fi
'''


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.script = self.root / "install.sh"
        subprocess.run([sys.executable, str(ROOT / "packaging/release-downloads.py"),
                        "install-script", "--output", str(self.script)], check=True)
        self.home = self.root / "a home with ' $ and % \\ and \""
        self.data = self.home / "custom data"
        self.bin = self.home / "custom bin"
        self.tools = self.root / "tools"
        self.tools.mkdir()
        self.release = self.root / "release"
        self.release.mkdir()
        self.image = self.release / ASSET
        self.image.write_text(APPIMAGE)
        self.write_checksums()
        for name, contents in {
            "curl": CURL,
            "uname": '''#!/bin/sh
case "$1" in
  -s) printf '%s\\n' "${FIXTURE_OS:-Linux}" ;;
  -m) printf '%s\\n' "${FIXTURE_ARCH:-x86_64}" ;;
  *) exit 1 ;;
esac
''',
        }.items():
            path = self.tools / name
            path.write_text(contents)
            path.chmod(0o755)
        self.env = dict(os.environ, HOME=str(self.home), XDG_DATA_HOME=str(self.data),
                        CINNABAR_BIN_DIR=str(self.bin), FIXTURE_ROOT=str(self.root),
                        FIXTURE_BASE=BASE, FIXTURE_TAG="v1.2.3",
                        PATH=f"{self.tools}:{os.environ['PATH']}")
        self.env.pop("APPIMAGE_EXTRACT_AND_RUN", None)

    def write_checksums(self, digest=None, copies=1):
        digest = digest or hashlib.sha256(self.image.read_bytes()).hexdigest()
        (self.release / CONFIG["checksums"]).write_text(f"{digest}  {ASSET}\n" * copies)

    def install(self, *args, **env):
        return subprocess.run(["sh", str(self.script), *args], env=dict(self.env, **env),
                              capture_output=True, text=True)

    def requests(self):
        path = self.root / "requests"
        return [json.loads(line)[-1] for line in path.read_text().splitlines()] if path.exists() else []

    def preserve_existing(self):
        image = self.data / "cinnabar/app/Cinnabar.AppImage"
        image.parent.mkdir(parents=True)
        image.write_bytes(b"previous install")
        data = self.data / "cinnabar/worlds/keep"
        data.parent.mkdir(parents=True)
        data.write_bytes(b"game data")
        return image, data

    def test_stable_install_resolves_one_tag_and_handles_quoted_paths(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.requests(), [BASE + "/latest",
                         f'{BASE}/download/v1.2.3/{CONFIG["checksums"]}',
                         f"{BASE}/download/v1.2.3/{ASSET}"])
        installed = self.data / "cinnabar/app/Cinnabar.AppImage"
        self.assertEqual(installed.read_bytes(), self.image.read_bytes())
        self.assertEqual((self.data / "icons/hicolor/256x256/apps/cinnabar.png").read_text(), "icon")
        desktop = (self.data / "applications/cinnabar.desktop").read_text()
        self.assertIn('Exec="', desktop)
        self.assertIn(r'\\$ and %% \\\\ and \\"', desktop)
        self.assertEqual((self.root / "runtime").read_text(), "extract\n")
        launched = subprocess.run([str(self.bin / "cinnabar"), "--argument", "two words"],
                                  env=self.env, capture_output=True, text=True)
        self.assertEqual(launched.returncode, 0, launched.stderr)
        self.assertEqual((self.root / "launch").read_text(),
                         "--appimage-extract-and-run\n--argument\ntwo words\n")
        self.assertFalse(list(self.data.rglob(".install.*")))

    def test_checksum_mismatch_preserves_app_and_data_without_executing_download(self):
        previous, data = self.preserve_existing()
        self.image.write_text(APPIMAGE + "# tampered\n")
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("SHA-256 mismatch", result.stderr)
        self.assertEqual(previous.read_bytes(), b"previous install")
        self.assertEqual(data.read_bytes(), b"game data")
        self.assertFalse((self.root / "runtime").exists())
        self.assertFalse(self.bin.exists())

    def test_missing_or_duplicate_checksum_never_downloads_or_replaces_app(self):
        previous, _ = self.preserve_existing()
        for copies in (0, 2):
            with self.subTest(copies=copies):
                self.write_checksums(copies=copies)
                result = self.install("--version", "v1.2.3")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("no unique valid checksum", result.stderr)
                self.assertEqual(previous.read_bytes(), b"previous install")
                self.assertNotIn(f"{BASE}/download/v1.2.3/{ASSET}", self.requests())

    def test_failed_download_and_extraction_preserve_existing_install(self):
        previous, _ = self.preserve_existing()
        for failure in ({"FIXTURE_FAIL_ASSET": ASSET}, {"FIXTURE_EXTRACT_FAIL": "1"}):
            with self.subTest(failure=failure):
                result = self.install(**failure)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(previous.read_bytes(), b"previous install")
                self.assertFalse(self.bin.exists())

    def test_unsupported_arch_and_invalid_tag_fail_before_network(self):
        for args, env in (((), {"FIXTURE_ARCH": "aarch64"}),
                          (("--version", "../other"), {})):
            with self.subTest(args=args, env=env):
                result = self.install(*args, **env)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.requests(), [])

    def test_macos_points_to_native_installer_without_network(self):
        result = self.install(FIXTURE_OS="Darwin")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("install the DMG", result.stderr)
        self.assertEqual(self.requests(), [])

    def test_no_stable_release_returns_useful_error(self):
        result = self.install(FIXTURE_NO_RELEASE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No stable release is available", result.stderr)
        self.assertFalse(self.data.exists())

    def test_nightly_and_exact_tag_do_not_query_latest(self):
        for args, tag in ((("--channel", "nightly"), "nightly"),
                          (("--version", "v2.0.1"), "v2.0.1")):
            with self.subTest(args=args):
                result = self.install(*args, FIXTURE_TAG=tag)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertNotIn(BASE + "/latest", self.requests())
                self.assertIn(f"{BASE}/download/{tag}/{ASSET}", self.requests())

    def test_successful_update_preserves_worlds_and_allows_mounted_runtime_opt_in(self):
        _, data = self.preserve_existing()
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(data.read_bytes(), b"game data")
        launched = subprocess.run([str(self.bin / "cinnabar"), "hello"],
                                  env=dict(self.env, APPIMAGE_EXTRACT_AND_RUN="0"),
                                  capture_output=True, text=True)
        self.assertEqual(launched.returncode, 0, launched.stderr)
        self.assertEqual((self.root / "launch").read_text(), "hello\n")


if __name__ == "__main__":
    unittest.main()
