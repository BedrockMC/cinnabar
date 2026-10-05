"""Exercise the SSH publication boundary with hostile archives and rollback."""

import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location("receive", Path(__file__).parents[1] / "deploy/receive.py")
RECEIVE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RECEIVE)


def archive(names, symlink=False):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w:gz") as tar:
        for name in names:
            entry = tarfile.TarInfo(name)
            if symlink:
                entry.type = tarfile.SYMTYPE
                entry.linkname = "/etc/passwd"
                tar.addfile(entry)
            else:
                entry.size = 2
                tar.addfile(entry, io.BytesIO(b"ok"))
    stream.seek(0)
    return stream


class DeploymentTests(unittest.TestCase):
    def test_publish_and_reject_hostile_payload_without_replacing_live_site(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            release = "a" * 40 + "-1-1"
            RECEIVE.deploy(root, release, archive(RECEIVE.FILES))
            current = (root / "current").resolve()
            self.assertEqual((current / "index.html").read_bytes(), b"ok")
            for names, symlink in [
                (["../outside"], False),
                (["index.html"], False),
                (RECEIVE.FILES, True),
                (["index.html", "index.html"], False),
            ]:
                with self.assertRaises(ValueError):
                    RECEIVE.deploy(root, "b" * 40 + "-2-1", archive(names, symlink))
                self.assertEqual((root / "current").resolve(), current)
                self.assertFalse((root / "releases" / ("b" * 40 + "-2-1")).exists())
            RECEIVE.deploy(root, "c" * 40 + "-3-1", archive(RECEIVE.FILES))
            self.assertNotEqual((root / "current").resolve(), current)
            self.assertTrue(current.exists())

    def test_release_identifier_cannot_escape_release_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(ValueError):
                RECEIVE.deploy(Path(temporary), "../escape", archive(RECEIVE.FILES))


if __name__ == "__main__":
    unittest.main()
