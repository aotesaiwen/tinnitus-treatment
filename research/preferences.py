"""Participant-local, exploratory music recipe recommendations (stdlib only)."""

import argparse
import json
import math
from pathlib import Path
import random


RECIPE_FIELDS = {
    "recipe_id", "provider", "style", "tempo_bpm", "duration_s", "prompt"
}
FEEDBACK_FIELDS = {"participant_id", "recipe_id", "rating", "discomfort"}
PRIOR_ALPHA = 2.0
PRIOR_BETA = 2.0


def _fields(value, required, label, optional=()):
    if not isinstance(value, dict):
        raise ValueError(f"{label} must be an object")
    missing = required - value.keys()
    extra = value.keys() - required - set(optional)
    if missing or extra:
        raise ValueError(
            f"{label}: missing fields {sorted(missing)}, unknown fields {sorted(extra)}"
        )


def _identifier(value, label):
    if (
        not isinstance(value, str) or not value.strip()
        or value != value.strip() or len(value) > 128
    ):
        raise ValueError(f"{label} must be a nonempty, trimmed string (max 128 characters)")


def _number(value, low, high, label, integer=False):
    if (
        isinstance(value, bool) or not isinstance(value, (int, float))
        or (integer and not isinstance(value, int))
        or not low <= value <= high or not math.isfinite(value)
    ):
        kind = "integer" if integer else "finite number"
        raise ValueError(f"{label} must be a {kind} between {low} and {high}")


def _validate(data):
    _fields(data, {"catalog", "feedback"}, "input", optional={"synthetic"})
    if "synthetic" in data and not isinstance(data["synthetic"], bool):
        raise ValueError("synthetic must be a boolean")
    if not isinstance(data["catalog"], list) or not data["catalog"]:
        raise ValueError("catalog must be a nonempty list")
    if not isinstance(data["feedback"], list):
        raise ValueError("feedback must be a list")
    recipes = {}
    for index, recipe in enumerate(data["catalog"]):
        label = f"catalog[{index}]"
        _fields(recipe, RECIPE_FIELDS, label)
        _identifier(recipe["recipe_id"], f"{label}.recipe_id")
        if recipe["recipe_id"] in recipes:
            raise ValueError(f"duplicate recipe_id: {recipe['recipe_id']}")
        if recipe["provider"] not in ("composer", "ace_step"):
            raise ValueError(f"{label}.provider must be composer or ace_step")
        if recipe["style"] not in ("drift", "focus", "unwind"):
            raise ValueError(f"{label}.style must be drift, focus or unwind")
        _number(recipe["tempo_bpm"], 45, 100, f"{label}.tempo_bpm", integer=True)
        _number(recipe["duration_s"], 30, 300, f"{label}.duration_s", integer=True)
        if not isinstance(recipe["prompt"], str) or len(recipe["prompt"]) > 500:
            raise ValueError(f"{label}.prompt must be a string of at most 500 characters")
        if recipe["provider"] == "composer" and recipe["prompt"].strip():
            raise ValueError(f"{label}.prompt must be empty for provider composer")
        recipes[recipe["recipe_id"]] = recipe
    for index, event in enumerate(data["feedback"]):
        label = f"feedback[{index}]"
        _fields(event, FEEDBACK_FIELDS, label)
        _identifier(event["participant_id"], f"{label}.participant_id")
        _identifier(event["recipe_id"], f"{label}.recipe_id")
        if event["recipe_id"] not in recipes:
            raise ValueError(f"{label} references unknown recipe_id: {event['recipe_id']}")
        _number(event["rating"], 1, 5, f"{label}.rating")
        if not isinstance(event["discomfort"], bool):
            raise ValueError(f"{label}.discomfort must be a boolean")
    return recipes


def recommend(data, participant_id, seed):
    """Return one recipe and an auditable score table; never generate or play audio."""
    _identifier(participant_id, "participant_id")
    if isinstance(seed, bool) or not isinstance(seed, int):
        raise ValueError("seed must be an integer")
    recipes = _validate(data)
    feedback = {recipe_id: [] for recipe_id in recipes}
    for event in data["feedback"]:
        if event["participant_id"] == participant_id:
            feedback[event["recipe_id"]].append(event)

    rng = random.Random(seed)
    candidates, excluded = [], []
    for recipe_id in sorted(recipes):
        events = feedback[recipe_id]
        discomfort_count = sum(event["discomfort"] for event in events)
        if discomfort_count:
            excluded.append({
                "recipe_id": recipe_id,
                "reason": "participant_reported_discomfort",
                "feedback_count": len(events),
                "discomfort_count": discomfort_count,
            })
            continue
        normalized = [(event["rating"] - 1) / 4 for event in events]
        alpha = PRIOR_ALPHA + math.fsum(normalized)
        beta = PRIOR_BETA + math.fsum(1 - rating for rating in normalized)
        candidates.append({
            "recipe_id": recipe_id,
            "feedback_count": len(events),
            "mean_rating": (
                math.fsum(event["rating"] for event in events) / len(events)
                if events else None
            ),
            "alpha": alpha,
            "beta": beta,
            "regularized_score": alpha / (alpha + beta),
            "exploration_sample": rng.betavariate(alpha, beta),
        })
    if not candidates:
        raise ValueError("all recipes excluded by this participant's discomfort reports")

    selected = max(candidates, key=lambda item: item["exploration_sample"])
    return {
        "scope": "preference_only_exploratory",
        "participant_id": participant_id,
        "seed": seed,
        "synthetic_input": data.get("synthetic", False),
        "method": "fractional_rating_beta_thompson",
        "prior": {"alpha": PRIOR_ALPHA, "beta": PRIOR_BETA},
        "feedback_count": sum(len(events) for events in feedback.values()),
        "eligible_recipe_count": len(candidates),
        "selected_recipe": dict(recipes[selected["recipe_id"]]),
        "reasons": [
            "Only the requested participant's ratings affect scores and exclusions.",
            "Any reported discomfort excludes that recipe for this participant.",
            "The largest seeded Beta sample selects a recipe, allowing exploration.",
            "Preference scores do not estimate treatment benefit or playback safety.",
        ],
        "candidates": candidates,
        "excluded": excluded,
    }


def _reject_constant(value):
    raise ValueError(f"nonfinite JSON number: {value}")


def _unique_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--participant", required=True)
    parser.add_argument("--seed", required=True, type=int)
    args = parser.parse_args()
    try:
        data = json.loads(
            args.input.read_text(encoding="utf-8"),
            parse_constant=_reject_constant,
            object_pairs_hook=_unique_keys,
        )
        result = recommend(data, args.participant, args.seed)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(json.dumps(result, indent=2, ensure_ascii=False, allow_nan=False))


if __name__ == "__main__":
    main()
