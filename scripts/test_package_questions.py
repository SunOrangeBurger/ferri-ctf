"""Checks for private/public separation and safe, complete local deliveries."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile

import package_questions as package


def fixture(path, extra=None):
    with zipfile.ZipFile(path, "w") as archive:
        for number in range(1, 16):
            prefix = f"FINAL_QUESTIONS/Question {number}/Example"
            participant = {name: b"public evidence" for name in
                           package.PARTICIPANT_FILES.get(number, {"README.md"})}
            participant["README.md"] = b"# Example\n\n**Difficulty:** Hard\n\nPublic question.\n"
            if number == 14:
                participant["README.md"] += (b"## Start here\n\nWebsite instructions.\n\n"
                                              b"`shattered_backup.zip` has evidence.\n\n"
                                              b"The hosted workbench gives budgets.\n\nRestoring sectors is deterministic.\n")
            for name, data in participant.items():
                archive.writestr(prefix + "_Participants/" + name, data)
            archive.writestr(prefix + "_backend/private.txt", "organizer secret")
            archive.writestr(prefix + "_Solution/README.md", "private solution")
            archive.writestr(prefix + "_backend/runtime/old.sqlite3", "old event state")
            archive.writestr(prefix + "_backend/.venv/stale.txt", "old environment")
            if number <= 10:
                archive.writestr(prefix + "_backend/challenge.py", f"TITLE='Example {number}'\nPORT={9100 + number}\n")
            else:
                archive.writestr(prefix + "_backend/requirements.txt", "fastapi==0.134.0\nuvicorn==0.41.0\n")
                archive.writestr(prefix + "_backend/flag.txt", f"nexus{{static_private_{number}}}")
        if extra:
            archive.writestr(*extra)


class PackagingTests(unittest.TestCase):
    def test_complete_ordered_delivery_keeps_private_material_out_of_uploads(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source.zip"
            fixture(source)
            destination = root / "questions"
            self.assertEqual(package.build(source, destination), 15)
            self.assertEqual([p.name for p in sorted((destination / "uploads").glob("*.zip"))],
                             [f"Q{n:02}-Participants.zip" for n in range(1, 16)])
            for number in range(1, 16):
                record = json.loads((destination / "organizer" / f"Q{number:02}" / "challenge.json").read_text())
                self.assertIsNone(record["points"])
                self.assertIsNone(record["wave"])
                self.assertFalse(record["difficulty_reviewed"])
                with zipfile.ZipFile(destination / "uploads" / record["attachment"]) as archive:
                    self.assertEqual(set(archive.namelist()), package.PARTICIPANT_FILES.get(number, {"README.md"}))
                    for name in archive.namelist():
                        self.assertNotIn(record["expected_flag"].encode(), archive.read(name))
                        self.assertNotIn(b"organizer secret", archive.read(name))
            self.assertFalse(any(destination.rglob("*.sqlite3")))
            self.assertFalse(any(destination.rglob(".venv")))
            with zipfile.ZipFile(root / "CTF-Participant-Uploads.zip") as archive:
                self.assertEqual(len(archive.namelist()), 15)
                self.assertTrue(all(n.startswith("uploads/Q") for n in archive.namelist()))
            self.assertTrue((root / "CTF-Organizer-Private.zip").exists())

    def test_source_python_is_parsed_without_execution(self):
        values = package.config_literals("TITLE = 'Safe title'\nPORT = 9101\nraise RuntimeError('must not execute')\n")
        self.assertEqual(values, {"TITLE": "Safe title", "PORT": 9101})

    def test_traversal_and_unexpected_participant_files_are_rejected(self):
        for extra in (("../escape.txt", "bad path"),
                      ("FINAL_QUESTIONS/Question 1/Example_Participants/flag.txt", "secret")):
            with self.subTest(extra=extra[0]), tempfile.TemporaryDirectory() as temporary:
                source = Path(temporary) / "source.zip"
                fixture(source, extra)
                with self.assertRaises(ValueError):
                    package.build(source, Path(temporary) / "questions")
                self.assertFalse((Path(temporary) / "CTF-Participant-Uploads.zip").exists())

    def test_exact_flag_in_participant_plaintext_prevents_public_archive(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source.zip"
            fixture(source)
            # An allowed evidence file still must not contain an organizer answer.
            with zipfile.ZipFile(source) as archive:
                entries = {i.filename: archive.read(i) for i in archive.infolist()}
            entries["FINAL_QUESTIONS/Question 11/Example_Participants/support_ticket.txt"] = b"nexus{static_private_11}"
            source.write_bytes(package.make_zip(entries))
            with self.assertRaisesRegex(ValueError, "organizer flag"):
                package.build(source, Path(temporary) / "questions")
            self.assertFalse((Path(temporary) / "CTF-Participant-Uploads.zip").exists())

    def test_failed_cli_publishes_no_partial_delivery_and_existing_delivery_is_retained(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source.zip"
            fixture(source, ("FINAL_QUESTIONS/Question 1/Example_Participants/extra.txt", "unreviewed"))
            command = [sys.executable, str(Path(package.__file__)), str(source), "--output", str(root / "delivery/questions")]
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(list((root / "delivery").iterdir()), [])
            fixture(source)
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            flag_path = root / "delivery/questions/organizer/Q01/challenge.json"
            original = flag_path.read_bytes()
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(flag_path.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
