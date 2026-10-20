"""Reproducible best-of-N selection using technical DSP checks, never relief scores."""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time


TARGET_LUFS = -23.0
PEAK_CEILING_DBTP = -1.0
STAGE_TIMEOUT_S = 900


@dataclass(frozen=True)
class Config:
    binary: str
    output: Path
    engine: str = "composer"
    style: str = "drift"
    duration: int = 30
    tempo: int = 60
    seed: int = 42
    candidates: int = 4
    center: float = 6000.0


def validate(config: Config) -> Path:
    """Validate before creating any experiment files."""
    for field, lower, upper in (
        ("duration", 30, 300),
        ("tempo", 45, 100),
        ("candidates", 1, 16),
        ("seed", 0, 2**32 - 1),
    ):
        value = getattr(config, field)
        if type(value) is not int or not lower <= value <= upper:
            raise ValueError(f"{field} must be an integer between {lower} and {upper}")
    if config.seed + config.candidates - 1 > 2**32 - 1:
        raise ValueError("seed + candidates - 1 must fit an unsigned 32-bit seed")
    if config.engine not in ("composer", "ace-step"):
        raise ValueError("engine must be composer or ace-step")
    if config.style not in ("drift", "focus", "unwind"):
        raise ValueError("style must be drift, focus or unwind")
    if (
        isinstance(config.center, bool)
        or not isinstance(config.center, (int, float))
        or not math.isfinite(config.center)
        or not 20 <= config.center <= 16000
    ):
        raise ValueError("center must be finite and between 20 and 16000 Hz")
    executable = shutil.which(config.binary)
    if executable is None:
        raise ValueError(f"executable not found or not executable: {config.binary}")
    binary = Path(executable).resolve()
    if not binary.is_file():
        raise ValueError(f"binary is not a file: {binary}")
    if os.path.lexists(config.output):
        raise ValueError(f"output must be a new directory: {config.output}")
    return binary


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_object(path: Path) -> dict:
    def reject_constant(value: str) -> None:
        raise ValueError(f"non-finite JSON value: {value}")

    def finite_float(value: str) -> float:
        parsed = float(value)
        if not math.isfinite(parsed):
            raise ValueError(f"non-finite JSON number: {value}")
        return parsed

    with path.open(encoding="utf-8") as stream:
        value = json.load(stream, parse_constant=reject_constant, parse_float=finite_float)
    if not isinstance(value, dict):
        raise ValueError(f"expected a JSON object in {path.name}")
    return value


def write_summary(path: Path, summary: dict) -> None:
    temporary = path.with_suffix(".tmp")
    temporary.write_text(
        json.dumps(summary, ensure_ascii=False, indent=2, allow_nan=False) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def invoke(command: list[str], directory: Path, stage: str) -> dict:
    started = time.perf_counter()
    result = {
        "command": command,
        "returncode": None,
        "stdout_path": f"{directory.name}/{stage}.stdout.txt",
        "stderr_path": f"{directory.name}/{stage}.stderr.txt",
        "error": None,
    }
    stdout = stderr = ""
    try:
        completed = subprocess.run(
            command, capture_output=True, text=True, encoding="utf-8",
            errors="replace", timeout=STAGE_TIMEOUT_S, check=False,
        )
        result["returncode"] = completed.returncode
        stdout, stderr = completed.stdout, completed.stderr
        if completed.returncode != 0:
            result["error"] = f"{stage} exited with status {completed.returncode}"
    except subprocess.TimeoutExpired as error:
        result["error"] = f"{stage} timed out after {STAGE_TIMEOUT_S} seconds"
        stdout, stderr = error.stdout or "", error.stderr or ""
    except OSError as error:
        result["error"] = f"{stage} could not start: {error}"
    for suffix, content in (("stdout", stdout), ("stderr", stderr)):
        if isinstance(content, bytes):
            content = content.decode("utf-8", errors="replace")
        (directory / f"{stage}.{suffix}.txt").write_text(content, encoding="utf-8")
    result["elapsed_s"] = time.perf_counter() - started
    return result


def numeric(value: object, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError(f"report {name} must be a finite number")
    return float(value)


def evaluate(report: dict, source_hash: str, output_hash: str) -> tuple[list[str], dict | None]:
    reasons = []
    if report.get("all_pass") is not True:
        reasons.append("report all_pass is not true")
    warnings = report.get("warnings")
    if not isinstance(warnings, list) or warnings:
        reasons.append("report warnings must be an empty list")
    checks = report.get("checks")
    if (
        not isinstance(checks, list)
        or not checks
        or any(not isinstance(check, dict) or check.get("pass") is not True for check in checks)
    ):
        reasons.append("report must contain nonempty, individually passing checks")
    if report.get("input_sha256") != source_hash:
        reasons.append("report input hash does not match preserved source")
    if report.get("output_sha256") != output_hash:
        reasons.append("report output hash does not match processed WAV")
    if report.get("mode") != "notch":
        reasons.append("report mode is not notch")
    try:
        target = numeric(report.get("target_lufs"), "target_lufs")
        ceiling = numeric(report.get("true_peak_ceiling_db"), "true_peak_ceiling_db")
        levels = report.get("output")
        if not isinstance(levels, dict):
            raise ValueError("report output levels are missing")
        loudness = numeric(levels.get("integrated_lufs"), "output.integrated_lufs")
        peak = numeric(levels.get("true_peak_dbtp"), "output.true_peak_dbtp")
        if target != TARGET_LUFS or ceiling != PEAK_CEILING_DBTP:
            reasons.append("report processing targets differ from the experiment recipe")
        metrics = {
            "loudness_error_lu": abs(loudness - target),
            "peak_headroom_db": ceiling - peak,
        }
        # Independently enforce the numerical tolerances used by the DSP pipeline.
        if metrics["loudness_error_lu"] > 0.1 or metrics["peak_headroom_db"] < -0.05:
            reasons.append("reported output levels exceed DSP tolerances")
    except ValueError as error:
        reasons.append(str(error))
        metrics = None
    return reasons, metrics


def attempt(config: Config, binary: Path, root: Path, index: int) -> dict:
    started = time.perf_counter()
    seed = config.seed + index
    candidate_id = f"candidate-{index + 1:02d}"
    directory = root / candidate_id
    directory.mkdir()
    source = directory / "source.wav"
    output = directory / "processed.wav"
    manifest_path = directory / "source.wav.generation.json"
    report_path = directory / "report.json"
    record = {
        "id": candidate_id,
        "seed": seed,
        "admitted": False,
        "failures": [],
        "recipe": {
            "engine": config.engine, "style": config.style,
            "duration_s": config.duration, "tempo_bpm": config.tempo, "seed": seed,
        },
        "source_path": f"{candidate_id}/source.wav",
        "processed_path": f"{candidate_id}/processed.wav",
        "manifest_path": f"{candidate_id}/source.wav.generation.json",
        "report_path": f"{candidate_id}/report.json",
        "source_sha256": None,
        "processed_sha256": None,
        "generation_manifest": None,
        "checks": None,
        "warnings": None,
        "all_pass": None,
        "metrics": None,
        "generate": None,
        "process": None,
    }
    try:
        record["generate"] = invoke([
            str(binary), "generate", "--engine", config.engine, "--style", config.style,
            "--duration", str(config.duration), "--tempo", str(config.tempo),
            "--seed", str(seed), "--output", str(source),
        ], directory, "generate")
        if source.is_file():
            record["source_sha256"] = sha256(source)
        if record["generate"]["error"]:
            record["failures"].append(record["generate"]["error"])
            return record
        if record["source_sha256"] is None:
            raise ValueError("generation succeeded without a source WAV")
        manifest = read_object(manifest_path)
        record["generation_manifest"] = manifest
        if manifest.get("source_sha256") != record["source_sha256"]:
            raise ValueError("generation manifest hash does not match preserved source")
        request = manifest.get("request")
        expected_request = {
            "style": config.style, "duration_s": config.duration,
            "tempo_bpm": config.tempo, "seed": seed, "prompt": "",
        }
        if request != expected_request:
            raise ValueError("generation manifest request differs from the candidate recipe")
        record["process"] = invoke([
            str(binary), "process", "--input", str(source), "--output", str(output),
            "--report", str(report_path), "--center", str(config.center),
            "--mode", "notch", "--target-lufs", str(TARGET_LUFS),
            "--tp-ceiling", str(PEAK_CEILING_DBTP), "--format", "f32",
            "--width-oct", "1", "--transition-oct", str(1 / 24),
            "--atten-db", "60", "--ripple-db", "0.1",
        ], directory, "process")
        if record["process"]["error"]:
            record["failures"].append(record["process"]["error"])
        if output.is_file():
            record["processed_sha256"] = sha256(output)
        report = read_object(report_path)
        record.update({key: report.get(key) for key in ("checks", "warnings", "all_pass")})
        if record["processed_sha256"] is None:
            raise ValueError("processing did not produce an output WAV")
        failures, metrics = evaluate(
            report, record["source_sha256"], record["processed_sha256"],
        )
        record["failures"].extend(failures)
        record["metrics"] = metrics
        if sha256(source) != record["source_sha256"]:
            record["failures"].append("source WAV changed during processing")
        record["admitted"] = not record["failures"]
    except (OSError, ValueError) as error:
        record["failures"].append(str(error))
    finally:
        record["elapsed_s"] = time.perf_counter() - started
    return record


def rank(attempts: list[dict]) -> list[dict]:
    """Exact ties use the smaller seed; rejected candidates can never be selected."""
    return sorted(
        (candidate for candidate in attempts if candidate["admitted"]),
        key=lambda candidate: (
            candidate["metrics"]["loudness_error_lu"],
            -candidate["metrics"]["peak_headroom_db"],
            candidate["seed"],
        ),
    )


def run(config: Config) -> dict:
    binary = validate(config)
    binary_hash = sha256(binary)
    root = config.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    started = time.perf_counter()
    recipe = asdict(config)
    recipe["binary"], recipe["output"] = str(binary), str(root)
    summary = {
        "schema_version": 1,
        "experiment": "technical-best-of-n-v1",
        "status": "running",
        "started_at_utc": datetime.now(timezone.utc).isoformat(),
        "recipe": recipe,
        "processing": {
            "mode": "notch", "center_hz": config.center, "width_oct": 1,
            "transition_oct": 1 / 24, "atten_db": 60, "ripple_db": 0.1,
            "target_lufs": TARGET_LUFS, "true_peak_ceiling_db": PEAK_CEILING_DBTP,
            "format": "f32",
        },
        "runtime": {
            "binary_sha256": binary_hash, "python": platform.python_version(),
            "platform": {
                "system": platform.system(), "release": platform.release(),
                "machine": platform.machine(),
            },
            "stage_timeout_s": STAGE_TIMEOUT_S,
        },
        "selection_rule": [
            "Both commands exit 0; matching waveform hashes; all_pass true; no warnings; every check passes.",
            "Smallest absolute output LUFS error, then greatest true-peak headroom, then smallest seed.",
        ],
        "interpretation": "Technical ranking only; no musical preference, comfort or clinical efficacy claim.",
        "attempts": [],
        "ranking": [],
        "selected": None,
        "elapsed_s": 0.0,
    }
    summary_path = root / "summary.json"
    write_summary(summary_path, summary)
    for index in range(config.candidates):
        summary["attempts"].append(attempt(config, binary, root, index))
        ordered = rank(summary["attempts"])
        summary["ranking"] = [item["id"] for item in ordered]
        summary["selected"] = ordered[0]["id"] if ordered else None
        summary["elapsed_s"] = time.perf_counter() - started
        write_summary(summary_path, summary)
    summary["status"] = "selected" if summary["selected"] else "no_admitted_candidates"
    summary["elapsed_s"] = time.perf_counter() - started
    write_summary(summary_path, summary)
    return summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/tmusic")
    parser.add_argument("--output", type=Path, required=True, help="new directory; existing paths are refused")
    parser.add_argument("--engine", choices=("composer", "ace-step"), default="composer")
    parser.add_argument("--style", choices=("drift", "focus", "unwind"), default="drift")
    parser.add_argument("--duration", type=int, default=30)
    parser.add_argument("--tempo", type=int, default=60)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--candidates", type=int, default=4)
    parser.add_argument("--center", type=float, default=6000.0)
    args = parser.parse_args(argv)
    try:
        summary = run(Config(**vars(args)))
    except (OSError, ValueError) as error:
        print(f"selection: {error}", file=sys.stderr)
        return 1
    print(f"Report: {args.output.resolve() / 'summary.json'}")
    print(f"Admitted: {len(summary['ranking'])}/{len(summary['attempts'])}")
    if summary["selected"]:
        print(f"Technical selection: {summary['selected']} (no perceptual or clinical inference)")
        return 0
    print("No candidate admitted; no selection made.", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
