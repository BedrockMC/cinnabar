"""Check release increments, lockfile identity, and fail-before-write behavior."""

import importlib.util
from pathlib import Path
import tempfile
import tomllib
import unittest

spec = importlib.util.spec_from_file_location("bump_version", Path(__file__).parents[1] / "bump-version.py")
bump_version = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bump_version)


class BumpVersionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "Cargo.toml").write_text('''[workspace]
members = ["app", "tool"]
[workspace.package]
version = "1.2.3" # This is the release source of truth.
edition = "2024"
[workspace.dependencies]
other = "1.2.3"
''')
        for name, version in (("app", "version.workspace = true"), ("tool", 'version = "1.2.3"')):
            member = self.root / name
            member.mkdir()
            (member / "Cargo.toml").write_text(f'[package]\nname = "{name}"\n{version}\n')
        (self.root / "Cargo.lock").write_text('''# Keep this comment and dependency pins.
version = 4

[[package]]
name = "app"
version = "1.2.3"
dependencies = [
 "tool",
]

[[package]]
name = "app"
version = "1.2.3"
source = "registry+https://example.test/index"
checksum = "pinned"

[[package]]
name = "tool"
version = "1.2.3"
dependencies = [
 "app 1.2.3",
 "app 1.2.3 (registry+https://example.test/index)",
]
''')

    def test_patch_increment(self):
        self.assert_increment("patch", "1.2.4")

    def test_minor_increment_resets_patch(self):
        self.assert_increment("minor", "1.3.0")

    def test_major_increment_resets_minor_and_patch(self):
        self.assert_increment("major", "2.0.0")

    def assert_increment(self, increment, expected):
        self.assertEqual(bump_version.bump_version(self.root, increment), expected)
        manifest = tomllib.loads((self.root / "Cargo.toml").read_text())
        self.assertEqual(manifest["workspace"]["package"]["version"], expected)
        self.assertEqual(manifest["workspace"]["dependencies"]["other"], "1.2.3")
        packages = tomllib.loads((self.root / "Cargo.lock").read_text())["package"]
        self.assertEqual(packages[0]["version"], expected)
        self.assertEqual(packages[1]["version"], "1.2.3")
        self.assertEqual(packages[1]["checksum"], "pinned")
        self.assertEqual(packages[2]["version"], "1.2.3")
        self.assertEqual(packages[2]["dependencies"], [
            f"app {expected}", "app 1.2.3 (registry+https://example.test/index)",
        ])
        self.assertIn("# This is the release source of truth.", (self.root / "Cargo.toml").read_text())
        self.assertTrue((self.root / "Cargo.lock").read_text().startswith("# Keep this comment"))

    def test_current_does_not_change_files(self):
        before = self.files()
        self.assertEqual(bump_version.bump_version(self.root, "current"), "1.2.3")
        self.assertEqual(self.files(), before)

    def test_stale_lock_fails_before_either_file_changes(self):
        lock = self.root / "Cargo.lock"
        lock.write_text(lock.read_text().replace('version = "1.2.3"', 'version = "1.2.2"', 1))
        before = self.files()
        with self.assertRaisesRegex(ValueError, "Cargo.lock"):
            bump_version.bump_version(self.root, "patch")
        self.assertEqual(self.files(), before)

    def test_prerelease_source_is_rejected(self):
        manifest = self.root / "Cargo.toml"
        manifest.write_text(manifest.read_text().replace('version = "1.2.3"', 'version = "1.2.3-dev"'))
        before = self.files()
        with self.assertRaisesRegex(ValueError, "stable semantic version"):
            bump_version.bump_version(self.root, "patch")
        self.assertEqual(self.files(), before)

    def files(self):
        return {name: (self.root / name).read_bytes() for name in ("Cargo.toml", "Cargo.lock")}


if __name__ == "__main__":
    unittest.main()
