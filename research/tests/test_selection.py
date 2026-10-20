import contextlib
from dataclasses import replace
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from research.selection import Config, main, rank, run, sha256


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.config = Config(
            binary=sys.executable, output=self.root / "experiment", candidates=3,
        )

    def fake_runner(self, behaviors=None):
        behaviors = behaviors or {}

        def runner(command, **kwargs):
            stage = command[1]
            output = Path(command[command.index("--output") + 1])
            candidate = int(output.parent.name.rsplit("-", 1)[1])
            behavior = behaviors.get(candidate, {})
            if stage == "generate":
                if behavior.get("generation_error"):
                    raise OSError("provider unavailable")
                if behavior.get("generation_timeout"):
                    raise subprocess.TimeoutExpired(command, 900, output=b"partial provider output")
                seed = int(command[command.index("--seed") + 1])
                output.write_bytes(f"preserved-source-{seed}".encode())
                manifest = {
                    "source_sha256": sha256(output),
                    "request": {
                        "style": command[command.index("--style") + 1],
                        "duration_s": int(command[command.index("--duration") + 1]),
                        "tempo_bpm": int(command[command.index("--tempo") + 1]),
                        "seed": seed, "prompt": "",
                    },
                }
                Path(str(output) + ".generation.json").write_text(json.dumps(manifest))
            else:
                source = Path(command[command.index("--input") + 1])
                output.write_bytes(b"processed-" + source.read_bytes())
                report = {
                    "mode": "notch", "all_pass": behavior.get("all_pass", True),
                    "warnings": behavior.get("warnings", []),
                    "checks": [{"id": "L1", "pass": behavior.get("check_pass", True)}],
                    "target_lufs": -23.0, "true_peak_ceiling_db": -1.0,
                    "input_sha256": behavior.get("input_hash", sha256(source)),
                    "output_sha256": sha256(output),
                    "output": {
                        "integrated_lufs": behavior.get("loudness", -23.0),
                        "true_peak_dbtp": behavior.get("peak", -10.0),
                    },
                }
                report_path = Path(command[command.index("--report") + 1])
                report_path.write_text(json.dumps(report))
                if behavior.get("malformed_report"):
                    report_path.write_text('{"all_pass":')
            return subprocess.CompletedProcess(
                command, behavior.get(f"{stage}_returncode", 0), "fixture stdout", "fixture stderr",
            )

        return runner

    def test_warnings_nonzero_exit_and_false_checks_are_rejected(self):
        behaviors = {
            1: {"warnings": ["inadequate source-band energy"]},
            2: {"process_returncode": 2},
            3: {"check_pass": False},
        }
        with patch("research.selection.subprocess.run", side_effect=self.fake_runner(behaviors)):
            summary = run(self.config)
        self.assertEqual(summary["status"], "no_admitted_candidates")
        self.assertIsNone(summary["selected"])
        self.assertEqual(summary["ranking"], [])
        self.assertTrue(all(item["failures"] for item in summary["attempts"]))
        self.assertEqual(summary["attempts"][0]["warnings"], behaviors[1]["warnings"])
        saved = json.loads((self.config.output / "summary.json").read_text())
        self.assertEqual(saved, summary)

    def test_all_pass_false_rejected_even_when_individual_checks_pass(self):
        with patch(
            "research.selection.subprocess.run",
            side_effect=self.fake_runner({1: {"all_pass": False}}),
        ):
            summary = run(replace(self.config, candidates=1))
        self.assertIsNone(summary["selected"])
        self.assertIn("report all_pass is not true", summary["attempts"][0]["failures"])

    def test_ranking_prefers_loudness_then_headroom_then_seed(self):
        behaviors = {
            1: {"loudness": -23.01, "peak": -20},
            2: {"loudness": -23, "peak": -5},
            3: {"loudness": -23, "peak": -10},
        }
        with patch("research.selection.subprocess.run", side_effect=self.fake_runner(behaviors)):
            summary = run(self.config)
        self.assertEqual(summary["ranking"], ["candidate-03", "candidate-02", "candidate-01"])
        self.assertEqual(summary["selected"], "candidate-03")
        tied = summary["attempts"][1].copy()
        tied.update(id="another", seed=100)
        self.assertEqual([item["seed"] for item in rank([tied, summary["attempts"][1]])], [43, 100])
        for item in summary["attempts"]:
            source = self.config.output / item["source_path"]
            self.assertEqual(sha256(source), item["source_sha256"])
            self.assertEqual(source.read_bytes(), f"preserved-source-{item['seed']}".encode())
            self.assertEqual(item["recipe"]["seed"], item["seed"])

    def test_provider_failure_does_not_stop_other_candidates(self):
        with patch(
            "research.selection.subprocess.run",
            side_effect=self.fake_runner({1: {"generation_error": True}}),
        ) as process:
            summary = run(self.config)
        self.assertEqual(len(summary["attempts"]), 3)
        self.assertFalse(summary["attempts"][0]["admitted"])
        self.assertIsNone(summary["attempts"][0]["process"])
        self.assertEqual(process.call_count, 5)
        self.assertEqual(summary["selected"], "candidate-02")

    def test_existing_output_directory_is_never_overwritten(self):
        self.config.output.mkdir()
        marker = self.config.output / "keep.txt"
        marker.write_text("untouched")
        with patch("research.selection.subprocess.run") as process:
            with self.assertRaisesRegex(ValueError, "new directory"):
                run(self.config)
        process.assert_not_called()
        self.assertEqual(marker.read_text(), "untouched")

    def test_timeout_and_malformed_report_preserve_other_attempts(self):
        with patch(
            "research.selection.subprocess.run",
            side_effect=self.fake_runner({
                1: {"generation_timeout": True}, 2: {"malformed_report": True},
            }),
        ):
            summary = run(self.config)
        self.assertEqual(summary["ranking"], ["candidate-03"])
        self.assertIn("timed out", summary["attempts"][0]["failures"][0])
        log = self.config.output / summary["attempts"][0]["generate"]["stdout_path"]
        self.assertEqual(log.read_text(), "partial provider output")
        self.assertTrue(summary["attempts"][1]["failures"])

    def test_dangling_output_symlink_is_refused(self):
        self.config.output.symlink_to(self.root / "missing-target", target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "new directory"):
            run(self.config)
        self.assertTrue(self.config.output.is_symlink())

    def test_invalid_parameters_create_no_artifacts(self):
        invalid = [
            {"candidates": 0}, {"candidates": 17}, {"duration": 29},
            {"duration": 301}, {"tempo": 44}, {"tempo": 101},
            {"seed": -1}, {"seed": 2**32 - 1}, {"seed": True},
            {"center": float("nan")}, {"center": float("inf")},
            {"center": 0}, {"center": 16001}, {"engine": "unknown"},
            {"style": "unknown"}, {"binary": str(self.root / "missing")},
        ]
        with patch("research.selection.subprocess.run") as process:
            for values in invalid:
                with self.subTest(values=values), self.assertRaises(ValueError):
                    run(replace(self.config, **values))
        process.assert_not_called()
        self.assertFalse(self.config.output.exists())

    def test_tampered_hash_and_nonfinite_metrics_cannot_be_selected(self):
        with patch(
            "research.selection.subprocess.run",
            side_effect=self.fake_runner({
                1: {"input_hash": "wrong"}, 2: {"peak": float("nan")}, 3: {"loudness": -24},
            }),
        ):
            summary = run(self.config)
        self.assertIsNone(summary["selected"])
        self.assertTrue(all(item["failures"] for item in summary["attempts"]))

    def test_cli_returns_two_when_nothing_passes(self):
        args = [
            "--binary", sys.executable, "--output", str(self.config.output), "--candidates", "1",
        ]
        with (
            patch("research.selection.subprocess.run", side_effect=self.fake_runner({1: {"all_pass": False}})),
            contextlib.redirect_stdout(io.StringIO()),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            self.assertEqual(main(args), 2)
        self.assertTrue((self.config.output / "summary.json").is_file())

    def test_cli_returns_one_for_setup_error(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(main(["--binary", "/missing/tmusic", "--output", str(self.config.output)]), 1)


if __name__ == "__main__":
    unittest.main()
