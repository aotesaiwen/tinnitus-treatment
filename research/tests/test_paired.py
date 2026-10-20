import collections
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from research.paired import balanced_orders, prepare, sha256


class PairedTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source.wav"
        self.source.write_bytes(b"fixture source")
        self.binary = self.root / "tmusic"
        self.binary.touch()

    def run_fixture(self, warning=False, wrong_hash=False, spread=False, malformed=False):
        def run(command, **_):
            output = Path(command[command.index("-o") + 1])
            report = Path(command[command.index("--report") + 1])
            mode = command[command.index("--mode") + 1]
            output.write_bytes(mode.encode())
            data = {
                "all_pass": True, "checks": [{"pass": True}],
                "warnings": ["insufficient energy"] if warning and mode == "notch" else [],
                "input_sha256": "wrong" if wrong_hash else sha256(self.source),
                "output_sha256": sha256(output),
                "output": {"integrated_lufs": -24 if spread and mode == "sham" else -23},
            }
            report.write_text(json.dumps([] if malformed else data))
            return subprocess.CompletedProcess(command, 0, "", "")
        return run

    def prepare(self, **options):
        with patch("research.paired.subprocess.run", side_effect=self.run_fixture(**options)):
            return prepare(self.binary, self.source, self.root / "result", 6000, 1500, 42)

    def test_all_orders_balance_position_and_transitions(self):
        orders = balanced_orders(["a", "b", "c"], 42)
        self.assertEqual(orders, balanced_orders(["a", "b", "c"], 42))
        self.assertEqual(len({tuple(order) for order in orders}), 6)
        for index in range(3):
            self.assertEqual(collections.Counter(order[index] for order in orders), {"a": 2, "b": 2, "c": 2})
        transitions = collections.Counter(pair for order in orders for pair in zip(order, order[1:]))
        self.assertEqual(set(transitions.values()), {2})

    def test_same_source_publishes_neutral_schedule_with_separate_mapping(self):
        result = self.prepare()
        self.assertTrue(result["accepted"])
        schedule = json.loads((self.root / "result/listening/schedule.json").read_text())
        text = json.dumps(schedule)
        for private in ["notch", "sham", "passthrough", "6000", "1500", str(self.source)]:
            self.assertNotIn(private, text)
        for clip in schedule["clips"]:
            self.assertEqual(sha256(self.root / "result/listening" / clip["file"]), clip["sha256"])
        self.assertEqual(self.source.read_bytes(), b"fixture source")
        key = json.loads((self.root / "result/private/key.json").read_text())
        self.assertEqual({row["mode"] for row in key["conditions"]}, {"notch", "sham", "passthrough"})

    def test_warning_prevents_publication_even_if_numeric_checks_pass(self):
        self.assertFalse(self.prepare(warning=True)["accepted"])
        self.assertFalse((self.root / "result/listening").exists())
        self.assertTrue((self.root / "result/private/key.json").exists())

    def test_wrong_source_hash_prevents_publication(self):
        self.assertFalse(self.prepare(wrong_hash=True)["accepted"])

    def test_unmatched_loudness_prevents_publication(self):
        self.assertFalse(self.prepare(spread=True)["accepted"])

    def test_malformed_report_fails_closed(self):
        self.assertFalse(self.prepare(malformed=True)["accepted"])

    def test_existing_output_is_never_overwritten(self):
        self.prepare()
        key = (self.root / "result/private/key.json").read_bytes()
        with self.assertRaises(FileExistsError):
            self.prepare()
        self.assertEqual((self.root / "result/private/key.json").read_bytes(), key)

    def test_dangling_symlink_is_not_a_new_output_directory(self):
        destination = self.root / "elsewhere"
        try:
            (self.root / "result").symlink_to(destination, target_is_directory=True)
        except OSError:
            self.skipTest("symlink creation is unavailable")
        with self.assertRaises(FileExistsError):
            self.prepare()
        self.assertFalse(destination.exists())

    def test_invalid_centers_do_not_create_output(self):
        for center, sham in [(float("nan"), 1500), (6000, float("inf")), (0, 1500), (6000, 4000)]:
            with self.assertRaises(ValueError):
                prepare(self.binary, self.source, self.root / "result", center, sham, 42)
        self.assertFalse((self.root / "result").exists())


if __name__ == "__main__":
    unittest.main()
