import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from research.preferences import recommend


ROOT = Path(__file__).resolve().parents[2]


def recipe(recipe_id, style="drift"):
    return {
        "recipe_id": recipe_id, "provider": "composer", "style": style,
        "tempo_bpm": 54, "duration_s": 60, "prompt": "",
    }


def event(recipe_id, rating, participant_id="alice", discomfort=False):
    return {
        "participant_id": participant_id, "recipe_id": recipe_id,
        "rating": rating, "discomfort": discomfort,
    }


class PreferenceTests(unittest.TestCase):
    def setUp(self):
        self.data = {
            "catalog": [recipe("a"), recipe("b", "focus")],
            "feedback": [event("a", 4)],
        }

    def test_no_cross_participant_leakage(self):
        baseline = recommend(self.data, "alice", 9)
        self.data["feedback"].extend([
            event("a", 1, "bob", discomfort=True), event("b", 5, "bob")
        ] * 40)
        self.assertEqual(baseline, recommend(self.data, "alice", 9))
        bob = recommend(self.data, "bob", 9)
        self.assertEqual(bob["selected_recipe"]["recipe_id"], "b")
        self.assertEqual(bob["feedback_count"], 80)

    def test_padded_desktop_ids_remain_distinct_from_unpadded_ids(self):
        baseline = recommend(self.data, "alice", 9)
        self.data["feedback"].extend([
            event("a", 1, " alice ", discomfort=True),
            event("b", 5, " alice "),
        ])
        self.assertEqual(baseline, recommend(self.data, "alice", 9))
        padded = recommend(self.data, " alice ", 9)
        self.assertEqual(padded["participant_id"], " alice ")
        self.assertEqual(padded["feedback_count"], 2)
        self.assertEqual(padded["selected_recipe"]["recipe_id"], "b")
        self.assertEqual(padded["excluded"][0]["recipe_id"], "a")

    def test_long_existing_desktop_ids_are_preserved(self):
        participant = "listener-" + "x" * 512
        self.data["feedback"] = [event("a", 5, participant)]
        result = recommend(self.data, participant, 9)
        self.assertEqual(result["participant_id"], participant)
        self.assertEqual(result["feedback_count"], 1)
        self.assertEqual(result["candidates"][0]["mean_rating"], 5)

    def test_seed_reproducibility_and_no_mutation(self):
        original = copy.deepcopy(self.data)
        first = recommend(self.data, "alice", 123)
        self.assertEqual(first, recommend(self.data, "alice", 123))
        self.assertNotEqual(
            first["candidates"], recommend(self.data, "alice", 456)["candidates"]
        )
        self.assertEqual(original, self.data)

    def test_catalog_order_does_not_change_seeded_selection(self):
        first = recommend(self.data, "alice", 123)
        self.data["catalog"].reverse()
        self.assertEqual(first, recommend(self.data, "alice", 123))

    def test_cold_start_uses_prior(self):
        result = recommend(self.data, "new_participant", 1)
        self.assertEqual(result["feedback_count"], 0)
        self.assertEqual(result["scope"], "preference_only_exploratory")
        for candidate in result["candidates"]:
            self.assertEqual(candidate["regularized_score"], 0.5)
            self.assertIsNone(candidate["mean_rating"])

    def test_discomfort_overrides_high_rating(self):
        self.data["feedback"] = [event("a", 5)] * 100 + [
            event("a", 5, discomfort=True)
        ]
        for seed in range(20):
            result = recommend(self.data, "alice", seed)
            self.assertEqual(result["selected_recipe"]["recipe_id"], "b")
            self.assertEqual(result["excluded"][0]["discomfort_count"], 1)
        self.data["feedback"].append(event("b", 3, discomfort=True))
        with self.assertRaisesRegex(ValueError, "all recipes excluded"):
            recommend(self.data, "alice", 0)

    def test_repeated_feedback_changes_selection_distribution(self):
        self.data["feedback"] = [event("a", 5), event("b", 1)] * 40
        selected = [
            recommend(self.data, "alice", seed)["selected_recipe"]["recipe_id"]
            for seed in range(100)
        ]
        self.assertGreaterEqual(selected.count("a"), 95)
        scores = recommend(self.data, "alice", 1)["candidates"]
        self.assertGreater(scores[0]["regularized_score"], 0.9)
        self.assertLess(scores[1]["regularized_score"], 0.1)

    def test_invalid_ratings(self):
        for rating in (0, 6, float("nan"), float("inf"), -float("inf"), True, "5", None):
            with self.subTest(rating=rating):
                self.data["feedback"][0]["rating"] = rating
                with self.assertRaisesRegex(ValueError, "rating"):
                    recommend(self.data, "alice", 0)

    def test_invalid_catalog_and_feedback(self):
        invalid_changes = [
            lambda data: data["catalog"].append(recipe("a")),
            lambda data: data["feedback"][0].update(recipe_id="unknown"),
            lambda data: data["feedback"][0].update(discomfort=1),
            lambda data: data["feedback"][0].update(participant_id=""),
            lambda data: data["feedback"][0].update(participant_id=" \t\n"),
            lambda data: data["feedback"][0].update(participant_id=5),
            lambda data: data["catalog"][0].update(recipe_id=" a"),
            lambda data: data["catalog"][0].update(recipe_id="a" * 129),
            lambda data: data["catalog"][0].update(tempo_bpm=True),
            lambda data: data["catalog"][0].update(duration_s=301),
            lambda data: data["catalog"][0].update(provider="unknown"),
            lambda data: data["catalog"][0].update(style="unknown"),
            lambda data: data["catalog"][0].update(prompt="x" * 501),
            lambda data: data["catalog"][0].update(notch_hz=6000),
            lambda data: data.update(catalog=[]),
            lambda data: data.update(feedback={}),
        ]
        for change in invalid_changes:
            data = copy.deepcopy(self.data)
            change(data)
            with self.subTest(data=data), self.assertRaises(ValueError):
                recommend(data, "alice", 0)

    def test_invalid_participant_and_seed(self):
        for participant, seed in (
            ("", 1), (" \t\n", 1), (None, 1), (5, 1),
            ("alice", True), ("alice", 1.5),
        ):
            with self.subTest(participant=participant, seed=seed):
                with self.assertRaises(ValueError):
                    recommend(self.data, participant, seed)

    def test_prompts_only_supported_by_ace_step(self):
        self.data["catalog"][0]["prompt"] = "Soft piano"
        with self.assertRaisesRegex(ValueError, "prompt must be empty for provider composer"):
            recommend(self.data, "alice", 42)
        self.data["catalog"][0]["provider"] = "ace_step"
        recommend(self.data, "alice", 42)
        self.data["catalog"][0].update(provider="composer", prompt=" \t\n")
        recommend(self.data, "alice", 42)

    def test_cli_fixture(self):
        result = subprocess.run(
            [sys.executable, "-m", "research.preferences", "--input",
             str(ROOT / "research/examples/preferences.json"),
             "--participant", "synthetic-alice", "--seed", "42"],
            cwd=ROOT, capture_output=True, text=True, check=True,
        )
        output = json.loads(result.stdout)
        self.assertTrue(output["synthetic_input"])
        self.assertEqual(output["feedback_count"], 4)
        self.assertEqual(output["excluded"][0]["recipe_id"], "focus-piano")

    def test_cli_rejects_nonfinite_and_duplicate_json(self):
        for raw in ('{"catalog": NaN}', '{"catalog": [], "catalog": []}'):
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "invalid.json"
                path.write_text(raw, encoding="utf-8")
                result = subprocess.run(
                    [sys.executable, "-m", "research.preferences", "--input",
                     str(path), "--participant", "alice", "--seed", "42"],
                    cwd=ROOT, capture_output=True, text=True,
                )
            self.assertEqual(result.returncode, 2)
            self.assertIn("error:", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
