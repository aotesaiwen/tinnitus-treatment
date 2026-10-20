import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from research.benchmark import run, validate_plan
from research.selection import read_object, sha256


class BenchmarkTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.output = self.root / "matrix"
        self.plan = {
            "recipes": [{
                "id": "test-drift", "engine": "composer", "style": "drift",
                "duration_s": 30, "tempo_bpm": 54, "prompt": "",
            }],
            "seed_splits": {"development": [42, 43, 44], "holdout": [142, 143, 144]},
            "centers_hz": [1000, 6000],
            "budgets": [1, 2, 4],
        }
        self.commands = []

    def fake_runner(self, generation=None, processing=None):
        generation, processing = generation or {}, processing or {}

        def runner(command, **kwargs):
            self.assertTrue((self.output / "plan.json").is_file())
            self.assertEqual(read_object(self.output / "summary.json")["status"], "running")
            self.commands.append(command)
            option = lambda name: command[command.index(name) + 1]
            output = Path(option("--output"))
            if command[1] == "generate":
                seed = int(option("--seed"))
                behavior = generation.get(seed, {})
                if behavior.get("timeout"):
                    raise subprocess.TimeoutExpired(command, 900, output=b"partial generation")
                if behavior.get("os_error"):
                    raise OSError("provider unavailable")
                if not behavior.get("missing_source"):
                    output.write_bytes(f"raw-{seed}".encode())
                manifest = {
                    "schema_version": 1,
                    "engine": {"composer": "Composer", "ace-step": "AceStep"}[option("--engine")],
                    "source_sha256": sha256(output) if output.exists() else None,
                    "request": {
                        "style": option("--style"), "duration_s": int(option("--duration")),
                        "tempo_bpm": int(option("--tempo")), "seed": seed,
                        "prompt": option("--prompt"),
                    },
                    "model": {"test_fixture": True},
                }
                if behavior.get("wrong_hash"):
                    manifest["source_sha256"] = "wrong"
                if behavior.get("wrong_engine"):
                    manifest["engine"] = "Unknown"
                if behavior.get("wrong_request"):
                    manifest["request"]["prompt"] = "Changed prompt"
                Path(str(output) + ".generation.json").write_text(json.dumps(manifest))
            else:
                source = Path(option("--input"))
                seed = int(source.parent.name.split("-")[1])
                center = float(option("--center"))
                behavior = processing.get((seed, center), {})
                if behavior.get("timeout"):
                    raise subprocess.TimeoutExpired(command, 900, output=b"partial processing")
                if behavior.get("os_error"):
                    raise OSError("processor unavailable")
                if behavior.get("mutate_source"):
                    source.write_bytes(b"mutated source")
                output.write_bytes(b"processed-" + source.read_bytes() + str(center).encode())
                report = {
                    "all_pass": behavior.get("all_pass", True),
                    "warnings": behavior.get("warnings", []),
                    "checks": [{"id": "L1", "pass": behavior.get("check_pass", True)}],
                    "input_sha256": behavior.get("input_hash", sha256(source)),
                    "output_sha256": behavior.get("output_hash", sha256(output)),
                    "mode": "notch", "target_lufs": -23.0, "true_peak_ceiling_db": -1.0,
                    "output": {"integrated_lufs": behavior.get("lufs", -23.0), "true_peak_dbtp": -10.0},
                    "design": {"spec": {
                        "centers_hz": [behavior.get("center", center)],
                        "width_oct": 1, "transition_oct": 1 / 24,
                        "stop_atten_db": 60, "pass_ripple_db": 0.1,
                    }},
                }
                if not behavior.get("missing_report"):
                    Path(option("--report")).write_text(
                        '{"all_pass":' if behavior.get("malformed_report") else json.dumps(report),
                    )
            return subprocess.CompletedProcess(
                command, behavior.get("returncode", 0), "fixture output", "fixture error",
            )

        return runner

    def execute(self, generation=None, processing=None, keep_processed=False):
        with (
            patch("research.selection.subprocess.run", side_effect=self.fake_runner(generation, processing)),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            return run(self.plan, sys.executable, self.output, keep_processed)

    def test_source_reused_for_all_centers_and_evidence_preserved(self):
        result = self.execute()
        self.assertEqual(len(self.commands), 6 * 3)
        self.assertEqual(result["source_attempts_completed"], 6)
        self.assertTrue(all(row["admitted"] == 3 for row in result["rows"]))
        self.assertEqual(read_object(self.output / "summary.json"), result)
        self.assertEqual(sha256(self.output / "plan.json"), result["plan_sha256"])
        self.assertTrue((self.output / "admission.csv").is_file())
        for attempt in result["attempts"]:
            source = self.output / attempt["source_path"]
            self.assertEqual(source.read_bytes(), f'raw-{attempt["seed"]}'.encode())
            self.assertEqual(sha256(source), attempt["source_sha256"])
            self.assertEqual(read_object(source.parent / "summary.json"), attempt)
            for center in attempt["centers"]:
                self.assertTrue(center["admitted"])
                self.assertEqual(
                    center["process"]["command"][center["process"]["command"].index("--input") + 1],
                    str(source),
                )
                self.assertFalse(center["processed_retained"])
                self.assertFalse((self.output / center["processed_path"]).exists())
                self.assertTrue((self.output / center["report_path"]).is_file())
                self.assertTrue((self.output / center["process"]["stdout_path"]).is_file())

    def test_failures_remain_in_denominator_and_first_admitted_blocks(self):
        result = self.execute(
            generation={42: {"returncode": 7}},
            processing={(142, 6000): {"warnings": ["low band energy"]}},
        )
        development = next(r for r in result["rows"] if r["split"] == "development")
        self.assertEqual((development["attempted"], development["generated"], development["admitted"]), (3, 2, 2))
        block_row = next(r for r in result["budget_rows"]
                         if r["split"] == "development" and r["center_hz"] == 1000 and r["budget"] == 2)
        self.assertEqual(block_row["completed_blocks"], 1)
        self.assertEqual(block_row["omitted_partial_seeds"], [44])
        self.assertEqual(block_row["blocks"][0]["selected_seed"], 43)
        self.assertEqual(block_row["blocks"][0]["attempts_consumed"], 2)
        oversize = next(r for r in result["budget_rows"] if r["budget"] == 4)
        self.assertEqual(oversize["completed_blocks"], 0)
        self.assertIsNone(oversize["admission_rate"])
        warned = next(r for r in result["rows"] if r["split"] == "holdout" and r["center_hz"] == 6000)
        self.assertEqual(warned["warning_counts"], {"low band energy": 1})
        self.assertEqual(warned["admitted"], 2)
        failed = result["attempts"][0]
        self.assertEqual(failed["generate"]["returncode"], 7)
        self.assertTrue(all(c["process"] is None for c in failed["centers"]))

    def test_timeouts_and_unavailable_provider_continue_and_keep_logs(self):
        result = self.execute(
            generation={42: {"timeout": True}, 43: {"os_error": True}},
            processing={(44, 1000): {"timeout": True}},
        )
        self.assertEqual(result["status"], "completed")
        self.assertEqual(len(result["attempts"]), 6)
        first = result["attempts"][0]
        self.assertIn("timed out", first["failures"][0])
        self.assertIsNone(first["generate"]["returncode"])
        self.assertEqual((self.output / first["generate"]["stdout_path"]).read_text(), "partial generation")
        self.assertFalse(result["attempts"][2]["centers"][0]["admitted"])
        self.assertTrue(result["attempts"][2]["centers"][1]["admitted"])

    def test_unverified_outputs_are_retained_and_never_admitted(self):
        result = self.execute(processing={
            (42, 1000): {"missing_report": True}, (42, 6000): {"malformed_report": True},
            (43, 1000): {"input_hash": "wrong"}, (43, 6000): {"output_hash": "wrong"},
            (44, 1000): {"lufs": float("nan")},
        })
        for attempt in result["attempts"][:2]:
            for case in attempt["centers"]:
                self.assertFalse(case["admitted"])
                self.assertTrue(case["processed_retained"])
        nonfinite = result["attempts"][2]["centers"][0]
        self.assertFalse(nonfinite["admitted"])
        self.assertTrue(nonfinite["processed_retained"])

    def test_exit_status_warnings_checks_and_wrong_center_reject(self):
        result = self.execute(processing={
            (42, 1000): {"returncode": 2}, (42, 6000): {"warnings": ["band unavailable"]},
            (43, 1000): {"check_pass": False}, (43, 6000): {"all_pass": False},
            (44, 1000): {"center": 3000}, (44, 6000): {"lufs": -24.0},
        })
        for attempt in result["attempts"][:3]:
            self.assertTrue(all(not case["admitted"] and case["failures"] for case in attempt["centers"]))
            self.assertTrue(all(not case["processed_retained"] for case in attempt["centers"]))

    def test_invalid_manifest_prevents_processing_and_counts_failure(self):
        result = self.execute(generation={
            42: {"wrong_hash": True}, 43: {"wrong_request": True}, 44: {"wrong_engine": True},
        })
        for attempt in result["attempts"][:3]:
            self.assertFalse(attempt["generated"])
            self.assertTrue(all(case["process"] is None for case in attempt["centers"]))
        self.assertTrue(all(row["admitted"] == 0 for row in result["rows"] if row["split"] == "development"))

    def test_source_mutation_invalidates_prior_center_admissions(self):
        result = self.execute(processing={(42, 6000): {"mutate_source": True}})
        first = result["attempts"][0]
        self.assertFalse(first["generated"])
        self.assertTrue(all(not case["admitted"] for case in first["centers"]))
        self.assertIn("preserved source WAV changed", first["failures"][0])

    def test_keep_processed_retains_valid_outputs(self):
        result = self.execute(keep_processed=True)
        self.assertTrue(all(case["processed_retained"] for a in result["attempts"] for case in a["centers"]))

    def test_ace_prompt_matches_manifest_and_command(self):
        self.plan["recipes"][0].update(engine="ace-step", prompt="Soft brushed cymbals and airy strings.")
        result = self.execute()
        self.assertTrue(all(a["generated"] for a in result["attempts"]))
        request = result["attempts"][0]["generation_manifest"]["request"]
        self.assertEqual(request["prompt"], self.plan["recipes"][0]["prompt"])

    def test_existing_directory_and_dangling_symlink_refused(self):
        self.output.mkdir()
        marker = self.output / "untouched"
        marker.write_text("retain")
        with patch("research.selection.subprocess.run") as process:
            with self.assertRaisesRegex(ValueError, "new directory"):
                run(self.plan, sys.executable, self.output)
            self.assertEqual(marker.read_text(), "retain")
            marker.unlink()
            self.output.rmdir()
            self.output.symlink_to(self.root / "missing", target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "new directory"):
                run(self.plan, sys.executable, self.output)
        process.assert_not_called()

    def test_invalid_plans_do_not_create_output(self):
        invalid = []
        for key, value in [
            ("id", "../escape"), ("duration_s", 29), ("duration_s", True),
            ("tempo_bpm", 101), ("prompt", "ignored by composer"), ("engine", "unknown"),
        ]:
            plan = copy.deepcopy(self.plan)
            plan["recipes"][0][key] = value
            invalid.append(plan)
        for key, value in [
            ("centers_hz", [float("nan")]), ("centers_hz", [float("inf")]),
            ("centers_hz", [10**1000]), ("centers_hz", [True]),
            ("centers_hz", [1000, 1000.0]), ("budgets", [0]), ("budgets", [1, 1]),
            ("seed_splits", {"development": [42], "holdout": [42]}),
            ("seed_splits", {"development": [42, 42], "holdout": [142]}),
            ("seed_splits", {"development": [2**32], "holdout": [142]}),
        ]:
            plan = copy.deepcopy(self.plan)
            plan[key] = value
            invalid.append(plan)
        duplicate = copy.deepcopy(self.plan)
        duplicate["recipes"].append({**duplicate["recipes"][0], "id": "different-id"})
        invalid.append(duplicate)
        with patch("research.selection.subprocess.run") as process:
            for plan in invalid:
                with self.subTest(plan=plan), self.assertRaises(ValueError):
                    run(plan, sys.executable, self.output)
                self.assertFalse(self.output.exists())
        process.assert_not_called()

    def test_sample_plan_has_expected_matrix_size(self):
        plan = read_object(Path(__file__).resolve().parents[1] / "examples/generation-matrix.json")
        validate_plan(plan)
        sources = len(plan["recipes"]) * sum(map(len, plan["seed_splits"].values()))
        self.assertEqual((sources, sources * len(plan["centers_hz"])), (72, 288))


if __name__ == "__main__":
    unittest.main()
