import json
from pathlib import Path
import tempfile
import unittest

from research.feedback import convert, export
from research.preferences import recommend


def event(event_id="event-a", participant="synthetic-a", seed=42, discomfort=False):
    return {
        "schema_version": 1, "event_id": event_id, "participant_id": participant,
        "track_id": "track-" + str(seed), "rating": 4, "discomfort": discomfort,
        "recipe": {"provider": "composer", "request": {
            "style": "drift", "duration_s": 30, "tempo_bpm": 54, "seed": seed, "prompt": "",
        }}, "created_at": "2026-10-21T10:00:00+08:00",
    }


class FeedbackImportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.ledger = self.root / "music_feedback.jsonl"

    def write(self, rows):
        self.ledger.write_text("".join(json.dumps(row) + "\n" for row in rows))

    def test_groups_variations_but_preserves_seed_provenance(self):
        self.write([event(), event("event-b", seed=43)])
        data, audit = convert(self.ledger, "synthetic-a")
        self.assertEqual(len(data["catalog"]), 1)
        self.assertEqual(len(data["feedback"]), 2)
        self.assertEqual([row["generation_seed"] for row in audit["events"]], [42, 43])
        self.assertEqual(recommend(data, "synthetic-a", 42)["feedback_count"], 2)

    def test_other_listener_cannot_change_catalog_or_exclusion(self):
        other = event("other", participant="synthetic-b", discomfort=True)
        other["recipe"]["request"]["style"] = "focus"
        self.write([event(), other])
        data, audit = convert(self.ledger, "synthetic-a")
        self.assertNotIn("synthetic-b", json.dumps([data, audit]))
        self.assertNotIn("focus", json.dumps(data))
        self.assertEqual(recommend(data, "synthetic-a", 42)["excluded"], [])

    def test_padded_profile_ids_export_without_merging_other_listeners(self):
        padded = " synthetic-a "
        self.write([
            event("padded", participant=padded),
            event("unpadded", participant="synthetic-a", discomfort=True),
        ])
        output = self.root / "padded-export"
        export(self.ledger, padded, output)
        data = json.loads((output / "preferences.json").read_text())
        audit = json.loads((output / "provenance.json").read_text())
        self.assertEqual(audit["participant_id"], padded)
        self.assertEqual(audit["included_events"], 1)
        self.assertEqual(audit["events"][0]["event_id"], "padded")
        self.assertEqual(data["feedback"][0]["participant_id"], padded)
        result = recommend(data, padded, 42)
        self.assertEqual(result["feedback_count"], 1)
        self.assertEqual(result["excluded"], [])
        unpadded, _ = convert(self.ledger, "synthetic-a")
        with self.assertRaisesRegex(ValueError, "all recipes excluded"):
            recommend(unpadded, "synthetic-a", 42)

    def test_long_profile_ids_export_and_recommend_without_normalization(self):
        participant = "listener-" + "x" * 512
        self.write([event(participant=participant)])
        output = self.root / "long-export"
        export(self.ledger, participant, output)
        data = json.loads((output / "preferences.json").read_text())
        audit = json.loads((output / "provenance.json").read_text())
        self.assertEqual(audit["participant_id"], participant)
        self.assertEqual(data["feedback"][0]["participant_id"], participant)
        result = recommend(data, participant, 42)
        self.assertEqual(result["participant_id"], participant)
        self.assertEqual(result["feedback_count"], 1)

    def test_invalid_participant_ids_are_rejected(self):
        self.write([event()])
        for participant in ("", " \t\n", None, 5):
            with self.subTest(participant=participant):
                with self.assertRaisesRegex(ValueError, "participant_id"):
                    convert(self.ledger, participant)

    def test_unknown_discomfort_is_skipped_instead_of_assumed_no(self):
        self.write([event(), event("unknown", discomfort=None)])
        data, audit = convert(self.ledger, "synthetic-a")
        self.assertEqual(len(data["feedback"]), 1)
        self.assertEqual(audit["unanswered_discomfort_skipped"], 1)
        self.write([event(discomfort=None)])
        with self.assertRaisesRegex(ValueError, "No usable responses"):
            convert(self.ledger, "synthetic-a")

    def test_reported_discomfort_remains_an_exclusion(self):
        self.write([event(discomfort=True)])
        data, _ = convert(self.ledger, "synthetic-a")
        with self.assertRaisesRegex(ValueError, "all recipes excluded"):
            recommend(data, "synthetic-a", 42)

    def test_duplicate_imports_do_not_count_twice(self):
        self.write([event(), event()])
        data, audit = convert(self.ledger, "synthetic-a")
        self.assertEqual(len(data["feedback"]), 1)
        self.assertEqual(audit["duplicate_events_skipped"], 1)
        changed = event()
        changed["rating"] = 1
        self.write([event(), changed])
        with self.assertRaisesRegex(ValueError, "conflicting duplicate"):
            convert(self.ledger, "synthetic-a")

    def test_duplicate_json_fields_cannot_reassign_feedback_or_hide_discomfort(self):
        original = json.dumps(event(discomfort=True))
        replacements = [
            ('"participant_id": "synthetic-a"',
             '"participant_id": "synthetic-b", "participant_id": "synthetic-a"'),
            ('"discomfort": true', '"discomfort": true, "discomfort": false'),
            ('"rating": 4', '"rating": 1, "rating": 4'),
            ('"event_id": "event-a"', '"event_id": "other", "event_id": "event-a"'),
            ('"seed": 42', '"seed": 7, "seed": 42'),
        ]
        for old, new in replacements:
            with self.subTest(field=old):
                self.ledger.write_text(original.replace(old, new) + "\n")
                with self.assertRaisesRegex(ValueError, "Feedback line 1: duplicate JSON field"):
                    convert(self.ledger, "synthetic-a")

    def test_nonfinite_json_values_are_rejected_during_parsing(self):
        original = json.dumps(event())
        for constant in ("NaN", "Infinity", "-Infinity"):
            with self.subTest(constant=constant):
                self.ledger.write_text(original.replace('"rating": 4', f'"rating": {constant}') + "\n")
                with self.assertRaisesRegex(ValueError, "Feedback line 1: nonfinite JSON number"):
                    convert(self.ledger, "synthetic-a")

    def test_invalid_events_fail_with_line_context(self):
        variants = []
        for name, value in [("schema_version", 2), ("schema_version", True), ("rating", True),
                            ("rating", 6), ("rating", float("nan")), ("created_at", "2026-10-21"),
                            ("discomfort", "no"), ("event_id", "")]:
            row = event()
            row[name] = value
            variants.append(row)
        row = event()
        row["recipe"]["request"]["prompt"] = "not supported by composer"
        variants.append(row)
        for row in variants:
            with self.subTest(row=row):
                self.write([row])
                with self.assertRaisesRegex(ValueError, "Feedback line 1"):
                    convert(self.ledger, "synthetic-a")

    def test_missing_listener_and_malformed_ledger_do_not_create_export(self):
        self.write([event()])
        with self.assertRaises(ValueError):
            export(self.ledger, "absent", self.root / "export")
        self.ledger.write_text('{"partial":')
        with self.assertRaises(ValueError):
            export(self.ledger, "synthetic-a", self.root / "export")
        self.assertFalse((self.root / "export").exists())

    def test_export_does_not_overwrite_previous_dataset(self):
        self.write([event()])
        output = self.root / "export"
        export(self.ledger, "synthetic-a", output)
        saved = (output / "preferences.json").read_bytes()
        self.write([event(discomfort=True)])
        with self.assertRaises(FileExistsError):
            export(self.ledger, "synthetic-a", output)
        self.assertEqual((output / "preferences.json").read_bytes(), saved)


if __name__ == "__main__":
    unittest.main()
