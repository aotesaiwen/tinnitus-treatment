"""Import one listener's desktop feedback into the preference research format."""

from __future__ import annotations

import argparse
from datetime import datetime
import hashlib
import json
from pathlib import Path

from research.preferences import (
    _reject_constant, _unique_keys, validate_input, validate_participant_id,
)
from research.selection import sha256, write_summary


def convert(ledger: Path, participant: str) -> tuple[dict, dict]:
    validate_participant_id(participant)
    catalog, events, seen, provenance = {}, [], {}, []
    skipped_unanswered = duplicates = 0
    with ledger.open(encoding="utf-8") as stream:
        for number, line in enumerate(stream, 1):
            try:
                row = json.loads(
                    line,
                    parse_constant=_reject_constant,
                    object_pairs_hook=_unique_keys,
                )
                if not isinstance(row, dict):
                    raise ValueError("each line must be an event object")
                if row.get("participant_id") != participant:
                    continue
                required = {"schema_version", "event_id", "participant_id", "track_id",
                            "recipe", "rating", "discomfort", "created_at"}
                if set(row) != required or type(row["schema_version"]) is not int or row["schema_version"] != 1:
                    raise ValueError("unsupported feedback event schema")
                for name in ("event_id", "track_id"):
                    if not isinstance(row[name], str) or not row[name].strip():
                        raise ValueError(f"{name} must be a nonempty string")
                if row["event_id"] in seen:
                    if row != seen[row["event_id"]]:
                        raise ValueError("conflicting duplicate event ID")
                    duplicates += 1
                    continue
                seen[row["event_id"]] = row
                timestamp = datetime.fromisoformat(row["created_at"].replace("Z", "+00:00"))
                if timestamp.utcoffset() is None:
                    raise ValueError("created_at must include a time zone")
                recipe = row["recipe"]
                if not isinstance(recipe, dict) or set(recipe) != {"provider", "request"}:
                    raise ValueError("recipe must contain provider and request")
                request = recipe["request"]
                if not isinstance(request, dict) or set(request) != {"style", "duration_s", "tempo_bpm", "seed", "prompt"}:
                    raise ValueError("unsupported generation request")
                if type(request["seed"]) is not int or not 0 <= request["seed"] <= 2**32 - 1:
                    raise ValueError("generation seed must be an unsigned 32-bit integer")
                creation = {"provider": recipe["provider"],
                            **{key: value for key, value in request.items() if key != "seed"}}
                canonical = json.dumps(creation, sort_keys=True, ensure_ascii=False, allow_nan=False)
                recipe_id = "recipe-" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()
                item = {"recipe_id": recipe_id, **creation}
                feedback = {"participant_id": participant, "recipe_id": recipe_id,
                            "rating": row["rating"], "discomfort": row["discomfort"]}
                if type(row["rating"]) is not int or not 1 <= row["rating"] <= 5:
                    raise ValueError("rating must be an integer from 1 to 5")
                if row["discomfort"] is not None and type(row["discomfort"]) is not bool:
                    raise ValueError("discomfort must be true, false or null")
                validate_input({"catalog": [item], "feedback": []})
                if row["discomfort"] is None:
                    skipped_unanswered += 1
                    continue
                catalog[recipe_id] = item
                events.append(feedback)
                provenance.append({"event_id": row["event_id"], "track_id": row["track_id"],
                                   "recipe_id": recipe_id, "generation_seed": request["seed"],
                                   "created_at": row["created_at"]})
            except (ValueError, TypeError, AttributeError) as error:
                raise ValueError(f"Feedback line {number}: {error}") from error
    if not events:
        raise ValueError("No usable responses for this listener; an explicit discomfort answer is required.")
    data = {"catalog": [catalog[key] for key in sorted(catalog)], "feedback": events}
    validate_input(data)
    audit = {"schema_version": 1, "participant_id": participant,
             "included_events": len(events), "recipe_count": len(catalog),
             "duplicate_events_skipped": duplicates,
             "unanswered_discomfort_skipped": skipped_unanswered,
             "grouping": "Identical provider, style, tempo, duration and prompt share a recipe; seeds remain in provenance.",
             "events": provenance}
    return data, audit


def export(ledger: Path, participant: str, output: Path) -> dict:
    if output.exists() or output.is_symlink():
        raise FileExistsError("Choose a new output directory.")
    before = sha256(ledger)
    data, audit = convert(ledger, participant)
    if sha256(ledger) != before:
        raise ValueError("Feedback changed during import; retry after the current save finishes.")
    output.mkdir(parents=True, exist_ok=False)
    write_summary(output / "preferences.json", data)
    write_summary(output / "provenance.json", audit)
    return {"included_events": audit["included_events"], "recipe_count": audit["recipe_count"],
            "unanswered_discomfort_skipped": audit["unanswered_discomfort_skipped"],
            "duplicate_events_skipped": audit["duplicate_events_skipped"],
            "preferences": str(output / "preferences.json"),
            "provenance": str(output / "provenance.json")}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--participant", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = export(args.ledger, args.participant, args.output)
    except (OSError, ValueError) as error:
        parser.exit(1, f"feedback: {error}\n")
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
