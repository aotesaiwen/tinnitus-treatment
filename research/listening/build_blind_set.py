#!/usr/bin/env python3
"""Package real audio as anonymous, equal-length, level-matched listening files.

Input is an explicit source list. Missing or short sources fail instead of being
replaced by another generator. Source identities are absent from manifest.json.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import subprocess

import numpy as np
from scipy.signal import resample_poly
import soundfile as sf

ROOT = Path(__file__).resolve().parents[2]
RATE = 44100
DURATION = 30


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source_list", type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "listening-demo/dist")
    args = parser.parse_args()
    spec = json.loads(args.source_list.read_text())
    output = args.output.resolve()
    (output / "audio").mkdir(parents=True, exist_ok=True)
    work = ROOT / "artifacts/audio/blind-20261021/prepared"
    work.mkdir(parents=True, exist_ok=True)
    manifest = dict(version=spec["version"], samples=[], method=spec["method"],
                    coverageNote=spec["coverageNote"], coverage=spec["coverage"],
                    attribution=spec["attribution"])
    reveal = dict(version=spec["version"], samples={})
    measurements = []
    for entry in spec["samples"]:
        raw = ROOT / entry["file"]
        if not raw.is_file():
            raise FileNotFoundError(raw)
        x, rate = sf.read(raw, dtype="float64", always_2d=True)
        if not np.isfinite(x).all() or x.shape[1] not in (1, 2):
            raise ValueError(f"Invalid input audio: {raw}")
        start = int(entry.get("start_seconds", 0) * rate)
        if start < 0 or len(x) - start < DURATION * rate:
            raise ValueError(f"Source must contain a full {DURATION}s excerpt: {raw}")
        x = x[start:start+DURATION*rate]
        if rate != RATE:
            common = math.gcd(rate, RATE)
            x = resample_poly(x, RATE//common, rate//common, axis=0)
        x = x[:RATE*DURATION]
        if x.shape[1] == 1:
            x = np.repeat(x, 2, axis=1)
        # Same short edge fades for every excerpt; no selected "best" segments.
        edge = int(.02 * RATE)
        x[:edge] *= np.linspace(0, 1, edge)[:, None]
        x[-edge:] *= np.linspace(1, 0, edge)[:, None]
        opaque = hashlib.sha256((digest(raw) + spec["version"]).encode()).hexdigest()[:20]
        if opaque in reveal["samples"]:
            raise ValueError("Duplicate source audio cannot masquerade as two models")
        cropped = work / f"{opaque}-crop.wav"
        matched = work / f"{opaque}-matched.wav"
        report = work / f"{opaque}-levels.json"
        sf.write(cropped, x, RATE, subtype="FLOAT")
        subprocess.run([str(ROOT/"target/release/tmusic"), "process", "-i", str(cropped),
                        "-o", str(matched), "--mode", "passthrough", "--center", "6000",
                        "--target-lufs=-23", "--tp-ceiling=-1", "--format", "f32",
                        "--report", str(report)], check=True, capture_output=True, text=True)
        levels = json.loads(report.read_text())
        if levels["design"] is not None or not levels["all_pass"]:
            raise ValueError(f"Unexpected processing report: {report}")
        y, sr = sf.read(matched, dtype="float64", always_2d=True)
        if abs(levels["output"]["integrated_lufs"] + 23) > .15:
            raise ValueError(f"Cannot loudness-match {raw} with gain alone")
        if sr != RATE or y.shape != (RATE*DURATION, 2):
            raise ValueError("Unexpected listening file shape")
        public = output/"audio"/f"{opaque}.flac"
        sf.write(public, y, RATE, format="FLAC", subtype="PCM_16")
        final, final_rate = sf.read(public, always_2d=True)
        if final.shape != y.shape or final_rate != RATE or not np.isfinite(final).all():
            raise ValueError("Invalid encoded listening file")
        if np.max(np.abs(final-y)) > 1/32768:
            raise ValueError("Unexpected lossless export error")
        rms = np.sqrt(np.mean(final.reshape(DURATION*10, RATE//10, 2)**2, axis=(1,2)))
        measurements.append(dict(id=opaque, source=entry["file"], source_sha256=digest(raw),
            listening_sha256=digest(public), rate=RATE, channels=2, duration=DURATION,
            levels=levels["output"], gain_db=levels["gain_db"],
            source_start_seconds=entry.get("start_seconds",0), edge_fade_seconds=.02,
            silent_tenths=int(np.count_nonzero(rms < 1e-5)), peak=float(np.max(np.abs(final))),
            bytes=public.stat().st_size))
        manifest["samples"].append(dict(id=opaque, audio=f"audio/{opaque}.flac", duration=DURATION))
        reveal["samples"][opaque] = {k: entry[k] for k in ["label", "shortLabel", "description", "configuration", "url"]}
    # Deterministic neutral ordering on disk; each browser session shuffles again.
    manifest["samples"].sort(key=lambda x:x["id"])
    for name, data in [("manifest.json", manifest), ("reveal.json", reveal)]:
        target=output/name
        temporary=target.with_suffix(".tmp")
        temporary.write_text(json.dumps(data, indent=2)+"\n")
        temporary.replace(target)
    (work/"measurements.json").write_text(json.dumps(measurements,indent=2)+"\n")
    print(json.dumps({"samples":len(measurements),"duration":DURATION,"rate":RATE,
                      "total_bytes":sum(m["bytes"] for m in measurements),"output":str(output)},indent=2))


if __name__=="__main__":
    main()
