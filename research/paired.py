"""Prepare same-source, counterbalanced listening material for technical review.

This prepares files, not a clinical protocol or a guarantee of perceptual blinding.
"""
from __future__ import annotations

import argparse
import hashlib
import itertools
import json
import math
from pathlib import Path
import platform
import random
import subprocess


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + "\n", encoding="utf-8")


def balanced_orders(labels: list[str], seed: int) -> list[list[str]]:
    """All six orders balance position and first-order transitions for three clips."""
    orders = [list(order) for order in itertools.permutations(labels)]
    random.Random(seed).shuffle(orders)
    return orders


def admitted(report: dict, returncode: int) -> bool:
    checks = report.get("checks")
    return (
        returncode == 0
        and report.get("all_pass") is True
        and report.get("warnings") == []
        and isinstance(checks, list) and bool(checks)
        and all(isinstance(check, dict) and check.get("pass") is True for check in checks)
    )


def prepare(binary: Path, source: Path, output: Path, center: float, sham_center: float, seed: int) -> dict:
    if not binary.is_file() or not source.is_file():
        raise ValueError("The tmusic binary and source WAV must exist.")
    if any(not math.isfinite(hz) or hz <= 0 for hz in [center, sham_center]):
        raise ValueError("Centers must be finite, positive frequencies.")
    # One-octave bands must not overlap; the DSP checks transitions and Nyquist too.
    if max(center, sham_center) / min(center, sham_center) <= 2:
        raise ValueError("Choose non-overlapping one-octave tinnitus and sham bands.")
    if not 0 <= seed <= 2**32 - 1:
        raise ValueError("Seed must be an unsigned 32-bit integer.")
    if output.exists() or output.is_symlink():
        raise FileExistsError(f"Output must be a new directory: {output}")
    binary, source, output = binary.resolve(), source.resolve(), output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    private = output / "private"
    private.mkdir()
    original_hash = sha256(source)
    modes = ["notch", "passthrough", "sham"]
    random.Random(seed).shuffle(modes)
    rows = []
    for index, mode in enumerate(modes, 1):
        label = f"clip-{index:02d}"
        destination = private / f"{label}.wav"
        report_path = private / f"{label}.report.json"
        command = [str(binary), "process", "-i", str(source), "-o", str(destination),
                   "--center", str(center), "--mode", mode, "--report", str(report_path)]
        if mode == "sham":
            command += ["--sham-center", str(sham_center)]
        error, report, code = None, {}, None
        try:
            result = subprocess.run(command, capture_output=True, text=True, timeout=300)
            code = result.returncode
            if report_path.exists():
                report = json.loads(report_path.read_text(encoding="utf-8"))
                if not isinstance(report, dict):
                    raise ValueError("The DSP report must be a JSON object.")
            if code:
                error = f"tmusic process exited with code {code}."
        except (OSError, subprocess.TimeoutExpired, ValueError) as exc:
            error = str(exc)
            report = {}
        levels = report.get("output", {})
        if not isinstance(levels, dict):
            levels = {}
        lufs = levels.get("integrated_lufs")
        valid_level = isinstance(lufs, (int, float)) and not isinstance(lufs, bool) and math.isfinite(lufs)
        hash_matches = (destination.is_file()
                        and report.get("input_sha256") == original_hash
                        and report.get("output_sha256") == sha256(destination))
        passed = admitted(report, code) and valid_level and hash_matches
        rows.append({"label": label, "mode": mode, "admitted": passed,
                     "command": command, "returncode": code,
                     "report": f"private/{report_path.name}", "source_sha256": original_hash,
                     "output_sha256": report.get("output_sha256"),
                     "output_lufs": lufs if valid_level else None,
                     "warnings": report.get("warnings", []), "error": error})
    levels = [row["output_lufs"] for row in rows if row["output_lufs"] is not None]
    spread = max(levels) - min(levels) if len(levels) == 3 else None
    accepted = (all(row["admitted"] for row in rows) and spread is not None and spread <= 0.2
                and sha256(source) == original_hash)
    key = {"schema_version": 1, "seed": seed, "center_hz": center, "sham_center_hz": sham_center,
           "binary_sha256": sha256(binary), "python_version": platform.python_version(),
           "source_sha256": original_hash, "accepted": accepted,
           "loudness_spread_lu": spread, "maximum_spread_lu": 0.2, "conditions": rows}
    write_json(private / "key.json", key)
    if accepted:
        listening = output / "listening"
        listening.mkdir()
        for row in rows:
            name = f'{row["label"]}.wav'
            (private / name).replace(listening / name)
        labels = [f'{row["label"]}.wav' for row in rows]
        write_json(listening / "schedule.json", {
            "schema_version": 1,
            "scope": "Technical listening review only; order balancing does not establish blinding or efficacy.",
            "orders": balanced_orders(labels, seed),
            "clips": [{"file": f'{row["label"]}.wav', "sha256": row["output_sha256"]} for row in rows],
        })
    summary = {"schema_version": 1, "accepted": accepted, "source_sha256": original_hash,
               "loudness_spread_lu": spread, "conditions_passed": sum(row["admitted"] for row in rows),
               "schedule": "listening/schedule.json" if accepted else None,
               "private_key": "private/key.json",
               "note": "Only share listening/ for review. private/ reveals the condition mapping."}
    write_json(output / "summary.json", summary)
    return summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/tmusic"))
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New output directory; existing paths are rejected.")
    parser.add_argument("--center", type=float, required=True)
    parser.add_argument("--sham-center", type=float, required=True)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()
    try:
        result = prepare(args.binary, args.source, args.output, args.center, args.sham_center, args.seed)
    except (OSError, ValueError) as exc:
        parser.exit(1, f"error: {exc}\n")
    print(json.dumps(result, indent=2))
    return 0 if result["accepted"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
