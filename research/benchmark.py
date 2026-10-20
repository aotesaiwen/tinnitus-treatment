"""Run a predeclared same-source generation matrix and measure technical admission."""

from __future__ import annotations

import argparse
from collections import Counter
import csv
from datetime import datetime, timezone
import math
import os
from pathlib import Path
import platform
import re
import shutil
import sys
import time

from research import selection


SLUG = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")
RECIPE_FIELDS = {"id", "engine", "style", "duration_s", "tempo_bpm", "prompt"}
PROCESSING = {
    "mode": "notch", "width_oct": 1, "transition_oct": 1 / 24,
    "atten_db": 60, "ripple_db": 0.1, "target_lufs": selection.TARGET_LUFS,
    "true_peak_ceiling_db": selection.PEAK_CEILING_DBTP, "format": "f32",
}


def integer(value: object, name: str, lower: int, upper: int) -> None:
    if type(value) is not int or not lower <= value <= upper:
        raise ValueError(f"{name} must be an integer between {lower} and {upper}")


def validate_plan(plan: dict) -> None:
    """Reject ambiguous or excessive plans before creating output files."""
    if not isinstance(plan, dict) or set(plan) != {
        "recipes", "seed_splits", "centers_hz", "budgets",
    }:
        raise ValueError("config requires exactly recipes, seed_splits, centers_hz and budgets")
    recipes = plan["recipes"]
    if not isinstance(recipes, list) or not 1 <= len(recipes) <= 32:
        raise ValueError("recipes must contain 1 to 32 entries")
    ids, signatures = set(), set()
    for recipe in recipes:
        if not isinstance(recipe, dict) or set(recipe) != RECIPE_FIELDS:
            raise ValueError(f"each recipe requires exactly {sorted(RECIPE_FIELDS)}")
        name = recipe["id"]
        if not isinstance(name, str) or len(name) > 80 or not SLUG.fullmatch(name):
            raise ValueError("recipe id must be a lowercase ASCII path slug, at most 80 characters")
        if name in ids:
            raise ValueError(f"duplicate recipe id: {name}")
        ids.add(name)
        if recipe["engine"] not in ("composer", "ace-step"):
            raise ValueError("engine must be composer or ace-step")
        if recipe["style"] not in ("drift", "focus", "unwind"):
            raise ValueError("style must be drift, focus or unwind")
        integer(recipe["duration_s"], "duration_s", 30, 300)
        integer(recipe["tempo_bpm"], "tempo_bpm", 45, 100)
        prompt = recipe["prompt"]
        if not isinstance(prompt, str) or len(prompt) > 500:
            raise ValueError("prompt must be a string of at most 500 characters")
        if recipe["engine"] == "composer" and prompt != "":
            raise ValueError("composer recipes require an empty prompt")
        signature = tuple(recipe[key] for key in sorted(RECIPE_FIELDS - {"id"}))
        if signature in signatures:
            raise ValueError("duplicate recipe settings under different ids")
        signatures.add(signature)
    splits = plan["seed_splits"]
    if not isinstance(splits, dict) or set(splits) != {"development", "holdout"}:
        raise ValueError("seed_splits requires development and holdout")
    seen = set()
    for split, seeds in splits.items():
        if not isinstance(seeds, list) or not 1 <= len(seeds) <= 128:
            raise ValueError(f"{split} requires 1 to 128 seeds")
        for seed in seeds:
            integer(seed, f"{split} seed", 0, 2**32 - 1)
            if seed in seen:
                raise ValueError("duplicate or overlapping split seeds would leak across comparisons")
            seen.add(seed)
    centers = plan["centers_hz"]
    if not isinstance(centers, list) or not 1 <= len(centers) <= 16:
        raise ValueError("centers_hz requires 1 to 16 frequencies")
    for center in centers:
        if (
            isinstance(center, bool) or not isinstance(center, (int, float))
            or not 20 <= center <= 16000 or not math.isfinite(center)
        ):
            raise ValueError("centers_hz must be finite numbers between 20 and 16000")
    if len(set(centers)) != len(centers):
        raise ValueError("duplicate centers_hz")
    budgets = plan["budgets"]
    if not isinstance(budgets, list) or not 1 <= len(budgets) <= 16:
        raise ValueError("budgets requires 1 to 16 entries")
    for budget in budgets:
        integer(budget, "budget", 1, 16)
    if len(set(budgets)) != len(budgets):
        raise ValueError("duplicate budgets")
    sources = len(recipes) * len(seen)
    if sources > 512 or sources * len(centers) > 4096:
        raise ValueError("plan exceeds 512 source attempts or 4096 processing checks")


def command_record(command: list[str], directory: Path, stage: str, root: Path) -> dict:
    result = selection.invoke(command, directory, stage)
    for suffix in ("stdout", "stderr"):
        result[f"{suffix}_path"] = str((directory / f"{stage}.{suffix}.txt").relative_to(root))
    return result


def process_source(
    binary: Path, root: Path, source_record: dict, center: float, keep_processed: bool,
) -> dict:
    source = root / source_record["source_path"]
    directory = source.parent / f"center-{center}"
    directory.mkdir()
    output, report_path = directory / "processed.wav", directory / "report.json"
    record = {
        "center_hz": center, "admitted": False, "failures": [], "process": None,
        "processed_path": str(output.relative_to(root)),
        "report_path": str(report_path.relative_to(root)), "processed_sha256": None,
        "hashes_verified": False, "processed_retained": False,
        "warnings": None, "checks": None, "all_pass": None, "metrics": None,
    }
    if not source_record["generated"]:
        record["failures"].append("source generation or manifest verification failed")
        return record
    try:
        record["process"] = command_record([
            str(binary), "process", "--input", str(source), "--output", str(output),
            "--report", str(report_path), "--center", str(center), "--mode", "notch",
            "--target-lufs", str(selection.TARGET_LUFS),
            "--tp-ceiling", str(selection.PEAK_CEILING_DBTP), "--format", "f32",
            "--width-oct", "1", "--transition-oct", str(1 / 24),
            "--atten-db", "60", "--ripple-db", "0.1",
        ], directory, "process", root)
        if record["process"]["error"]:
            record["failures"].append(record["process"]["error"])
        if output.is_file():
            record["processed_sha256"] = selection.sha256(output)
        report = selection.read_object(report_path)
        record.update({key: report.get(key) for key in ("warnings", "checks", "all_pass")})
        if record["processed_sha256"] is None:
            raise ValueError("processing did not produce an output WAV")
        current_source_hash = selection.sha256(source)
        record["hashes_verified"] = (
            current_source_hash == source_record["source_sha256"]
            and report.get("input_sha256") == source_record["source_sha256"]
            and report.get("output_sha256") == record["processed_sha256"]
        )
        failures, record["metrics"] = selection.evaluate(
            report, source_record["source_sha256"], record["processed_sha256"],
        )
        record["failures"].extend(failures)
        expected_spec = {
            "centers_hz": [center], "width_oct": 1, "transition_oct": 1 / 24,
            "stop_atten_db": 60, "pass_ripple_db": 0.1,
        }
        design = report.get("design")
        if not isinstance(design, dict) or design.get("spec") != expected_spec:
            record["failures"].append("report filter specification differs from the planned center/recipe")
        if current_source_hash != source_record["source_sha256"]:
            record["failures"].append("source WAV changed during processing")
        record["admitted"] = not record["failures"]
        # Only this newly-created result is eligible for removal. Preserve
        # malformed/missing-report and hash-mismatch artifacts for diagnosis.
        if not keep_processed and record["hashes_verified"]:
            output.unlink()
    except (OSError, ValueError) as error:
        record["failures"].append(str(error))
        record["admitted"] = False
    record["processed_retained"] = output.is_file()
    return record


def attempt_source(
    binary: Path, root: Path, recipe: dict, split: str, seed: int,
    centers: list, keep_processed: bool,
) -> dict:
    started = time.perf_counter()
    directory = root / recipe["id"] / split / f"seed-{seed}"
    directory.mkdir(parents=True)
    source = directory / "source.wav"
    manifest_path = directory / "source.wav.generation.json"
    record = {
        "recipe_id": recipe["id"], "split": split, "seed": seed,
        "generated": False, "failures": [], "generate": None,
        "source_path": str(source.relative_to(root)),
        "manifest_path": str(manifest_path.relative_to(root)),
        "source_sha256": None, "generation_manifest": None, "centers": [],
    }
    try:
        record["generate"] = command_record([
            str(binary), "generate", "--engine", recipe["engine"], "--style", recipe["style"],
            "--duration", str(recipe["duration_s"]), "--tempo", str(recipe["tempo_bpm"]),
            "--seed", str(seed), "--prompt", recipe["prompt"], "--output", str(source),
        ], directory, "generate", root)
        if source.is_file():
            record["source_sha256"] = selection.sha256(source)
        if record["generate"]["error"]:
            raise ValueError(record["generate"]["error"])
        if record["source_sha256"] is None:
            raise ValueError("generation succeeded without a source WAV")
        manifest = selection.read_object(manifest_path)
        record["generation_manifest"] = manifest
        expected = {key: recipe[key] for key in ("style", "duration_s", "tempo_bpm", "prompt")}
        expected["seed"] = seed
        if manifest.get("source_sha256") != record["source_sha256"]:
            raise ValueError("generation manifest hash does not match preserved source")
        if manifest.get("request") != expected:
            raise ValueError("generation manifest request differs from the recipe")
        if manifest.get("engine") != {"composer": "Composer", "ace-step": "AceStep"}[recipe["engine"]]:
            raise ValueError("generation manifest engine differs from the recipe")
        record["generated"] = True
    except (OSError, ValueError) as error:
        record["failures"].append(str(error))
    for center in centers:
        record["centers"].append(process_source(binary, root, record, center, keep_processed))
    if record["generated"]:
        try:
            if selection.sha256(source) != record["source_sha256"]:
                raise ValueError("preserved source WAV changed during the matrix")
        except (OSError, ValueError) as error:
            record["generated"] = False
            record["failures"].append(str(error))
            for case in record["centers"]:
                case["admitted"] = False
                case["failures"].append(str(error))
    record["elapsed_s"] = time.perf_counter() - started
    return record


def stage_seconds(stage: dict | None) -> float:
    return stage["elapsed_s"] if stage else 0.0


def tables(plan: dict, attempts: list[dict]) -> tuple[list[dict], list[dict]]:
    """Each source is one observation within a center, never an independent center replicate."""
    rows, budget_rows = [], []
    indexed = {(a["recipe_id"], a["split"], a["seed"]): a for a in attempts}
    for recipe in plan["recipes"]:
        for split, seeds in plan["seed_splits"].items():
            available = [indexed[(recipe["id"], split, s)] for s in seeds
                         if (recipe["id"], split, s) in indexed]
            for center in plan["centers_hz"]:
                cases = {a["seed"]: next(c for c in a["centers"] if c["center_hz"] == center)
                         for a in available}
                warnings = Counter()
                for case in cases.values():
                    if isinstance(case["warnings"], list):
                        warnings.update(str(warning) for warning in case["warnings"])
                admitted = sum(case["admitted"] for case in cases.values())
                rows.append({
                    "recipe_id": recipe["id"], "split": split, "center_hz": center,
                    "planned": len(seeds), "attempted": len(available),
                    "generated": sum(a["generated"] for a in available), "admitted": admitted,
                    "admission_rate": admitted / len(available) if available else None,
                    "generation_s": sum(stage_seconds(a["generate"]) for a in available),
                    "processing_s": sum(stage_seconds(case["process"]) for case in cases.values()),
                    "warning_cases": sum(bool(case["warnings"]) for case in cases.values()),
                    "warning_counts": dict(sorted(warnings.items())),
                    "missing_report_cases": sum(case["all_pass"] is None for case in cases.values()),
                })
                for budget in plan["budgets"]:
                    blocks = []
                    full_count = len(seeds) // budget
                    for block_index in range(full_count):
                        block_seeds = seeds[block_index * budget:(block_index + 1) * budget]
                        if not all(s in cases for s in block_seeds):
                            continue
                        selected = next((s for s in block_seeds if cases[s]["admitted"]), None)
                        consumed = block_seeds if selected is None else block_seeds[:block_seeds.index(selected) + 1]
                        blocks.append({
                            "seeds": block_seeds, "selected_seed": selected,
                            "admitted": selected is not None, "attempts_consumed": len(consumed),
                            "estimated_serial_s": sum(
                                stage_seconds(indexed[(recipe["id"], split, s)]["generate"])
                                + stage_seconds(cases[s]["process"]) for s in consumed
                            ),
                        })
                    successes = sum(block["admitted"] for block in blocks)
                    budget_rows.append({
                        "recipe_id": recipe["id"], "split": split, "center_hz": center,
                        "budget": budget, "planned_full_blocks": full_count,
                        "completed_blocks": len(blocks), "admitted_blocks": successes,
                        "admission_rate": successes / len(blocks) if blocks else None,
                        "incomplete_blocks": full_count - len(blocks),
                        "omitted_partial_seeds": seeds[full_count * budget:],
                        "estimated_serial_s": sum(block["estimated_serial_s"] for block in blocks),
                        "attempts_consumed": sum(block["attempts_consumed"] for block in blocks),
                        "blocks": blocks,
                    })
    return rows, budget_rows


def export_csv(path: Path, rows: list[dict], excluded: set[str]) -> None:
    if not rows:
        return
    with path.open("w", newline="", encoding="utf-8") as stream:
        fields = [key for key in rows[0] if key not in excluded]
        writer = csv.DictWriter(stream, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)


def run(plan: dict, binary: str, output: Path, keep_processed: bool = False) -> dict:
    validate_plan(plan)
    executable = shutil.which(binary)
    if executable is None:
        raise ValueError(f"executable not found or not executable: {binary}")
    binary_path = Path(executable).resolve()
    if not binary_path.is_file():
        raise ValueError(f"binary is not a file: {binary_path}")
    if os.path.lexists(output):
        raise ValueError(f"output must be a new directory: {output}")
    root = output.resolve()
    source_count = len(plan["recipes"]) * sum(map(len, plan["seed_splits"].values()))
    declaration = {
        "schema_version": 1, "experiment": "generation-matrix-v1",
        "created_at_utc": datetime.now(timezone.utc).isoformat(), "config": plan,
        "processing": PROCESSING, "source_attempts_planned": source_count,
        "center_checks_planned": source_count * len(plan["centers_hz"]),
        "execution_order": "recipe list, split object insertion order, seed list, center list",
        "keep_processed": keep_processed,
        "admission_rule": "Generation and processing exit 0; manifests and waveform hashes match; all checks pass; no warnings.",
        "budget_rule": "First admitted seed in each disjoint ordered full block; failed generations consume one attempt.",
        "interpretation": "Technical admission only. A local predeclared holdout split is not external preregistration or listening/clinical validation.",
        "runtime": {
            "binary": str(binary_path), "binary_sha256": selection.sha256(binary_path),
            "code_sha256": {
                "research/benchmark.py": selection.sha256(Path(__file__)),
                "research/selection.py": selection.sha256(Path(selection.__file__)),
            },
            "python": platform.python_version(),
            "platform": {"system": platform.system(), "release": platform.release(), "machine": platform.machine()},
            "stage_timeout_s": selection.STAGE_TIMEOUT_S,
        },
    }
    root.mkdir(parents=True, exist_ok=False)
    selection.write_summary(root / "plan.json", declaration)
    summary = {
        "schema_version": 1, "experiment": declaration["experiment"], "status": "running",
        "plan_path": "plan.json", "plan_sha256": selection.sha256(root / "plan.json"),
        "attempts": [], "rows": [], "budget_rows": [], "elapsed_s": 0.0,
        "source_attempts_planned": source_count, "source_attempts_completed": 0,
        "center_checks_planned": declaration["center_checks_planned"],
    }
    started = time.perf_counter()

    def checkpoint() -> None:
        summary["rows"], summary["budget_rows"] = tables(plan, summary["attempts"])
        summary["source_attempts_completed"] = len(summary["attempts"])
        summary["generation_s"] = sum(stage_seconds(a["generate"]) for a in summary["attempts"])
        summary["processing_s"] = sum(stage_seconds(c["process"]) for a in summary["attempts"] for c in a["centers"])
        summary["elapsed_s"] = time.perf_counter() - started
        selection.write_summary(root / "summary.json", summary)

    checkpoint()
    try:
        for recipe in plan["recipes"]:
            for split, seeds in plan["seed_splits"].items():
                for seed in seeds:
                    record = attempt_source(
                        binary_path, root, recipe, split, seed, plan["centers_hz"], keep_processed,
                    )
                    summary["attempts"].append(record)
                    selection.write_summary(root / Path(record["source_path"]).parent / "summary.json", record)
                    checkpoint()
                    admitted = sum(case["admitted"] for case in record["centers"])
                    print(f'{len(summary["attempts"])}/{source_count} {recipe["id"]}/{split}/{seed}: '
                          f'generated={record["generated"]}, admitted={admitted}/{len(plan["centers_hz"])}',
                          flush=True)
    except KeyboardInterrupt:
        summary["status"] = "interrupted"
        checkpoint()
        raise
    summary["status"] = "completed"
    checkpoint()
    export_csv(root / "admission.csv", summary["rows"], {"warning_counts"})
    export_csv(root / "budgets.csv", summary["budget_rows"], {"blocks", "omitted_partial_seeds"})
    return summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--binary", default="target/release/tmusic")
    parser.add_argument("--output", type=Path, required=True, help="New directory; existing paths are refused")
    parser.add_argument("--keep-processed", action="store_true")
    args = parser.parse_args(argv)
    try:
        summary = run(selection.read_object(args.config), args.binary, args.output, args.keep_processed)
    except (OSError, ValueError) as error:
        print(f"benchmark: {error}", file=sys.stderr)
        return 1
    print(f'Report: {args.output.resolve() / "summary.json"}')
    # Completion does not imply admission; code 2 exposes an entirely rejected run.
    return 0 if any(row["admitted"] for row in summary["rows"]) else 2


if __name__ == "__main__":
    raise SystemExit(main())
