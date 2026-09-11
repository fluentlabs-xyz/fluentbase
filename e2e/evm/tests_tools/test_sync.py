import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import sync_tests


class FixtureSyncTests(unittest.TestCase):
    def archive(self, root, member="fixtures/state_tests/for_prague/example.json"):
        archive = root / "fixtures.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            entry = tarfile.TarInfo(member)
            entry.size = 2
            bundle.addfile(entry, io.BytesIO(b"{}"))
        config = {
            "release": "test-release",
            "url": "unused",
            "sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
            "directory": "tests/pinned-fixtures",
            "forks": ["Prague"],
        }
        return archive, config

    def test_existing_legacy_directory_does_not_suppress_update(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            legacy = root / "tests/GeneralStateTests/local-work.json"
            legacy.parent.mkdir(parents=True)
            legacy.write_text("keep local data")
            archive, config = self.archive(root)
            (root / "ethereum-tests.json").write_text(json.dumps(config))
            with patch.object(sync_tests, "ROOT", root), patch(
                "sys.argv", ["sync_tests.py", "--archive", str(archive)]
            ):
                sync_tests.main()
                sync_tests.main()
            self.assertEqual(legacy.read_text(), "keep local data")
            self.assertTrue((root / config["directory"] / "state_tests/for_prague/example.json").is_file())

    def test_checksum_mismatch_does_not_install(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive, config = self.archive(root)
            config["sha256"] = "0" * 64
            destination = root / "output"
            destination.mkdir()
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                sync_tests.install(archive, destination, config)
            self.assertEqual(list(destination.iterdir()), [])

    def test_archive_without_supported_forks_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive, config = self.archive(root, "fixtures/state_tests/for_cancun/example.json")
            destination = root / "output"
            destination.mkdir()
            with self.assertRaisesRegex(ValueError, "no state tests"):
                sync_tests.install(archive, destination, config)
            self.assertFalse((destination / ".release.json").exists())

    def test_archive_paths_cannot_escape_destination(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive, config = self.archive(root, "fixtures/state_tests/for_prague/../../../escape.json")
            destination = root / "output"
            destination.mkdir()
            with self.assertRaisesRegex(ValueError, "Invalid fixture path"):
                sync_tests.install(archive, destination, config)
            self.assertFalse((root / "escape.json").exists())


if __name__ == "__main__":
    unittest.main()
