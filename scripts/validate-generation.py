"""Real-model smoke/QA matrix using the same Rust adapter and DSP as the app.

Build first: cargo build --release --features generation-http
Run: python3 scripts/validate-generation.py
Audio/reports remain in ignored artifacts/audio/. Small-sample technical QA only.
"""
import argparse
import datetime
import json
import pathlib
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]


def run(args):
    start = time.monotonic()
    result = subprocess.run(args, cwd=ROOT, capture_output=True, text=True)
    return result, round(time.monotonic() - start, 3)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=pathlib.Path, default=ROOT / "target/release/tmusic")
    parser.add_argument("--output", type=pathlib.Path, default=ROOT / "artifacts/audio" / datetime.datetime.now().strftime("%Y%m%d-%H%M%S"))
    parser.add_argument("--seeds", type=int, nargs="+", default=[42, 43])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    summary = {
        "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "scope": "Small-sample technical integration and source-suitability checks, not clinical or perceptual validation.",
        "sources": [], "checks": [],
    }
    revision = ROOT / ".runtime/model-revision.json"
    if revision.exists():
        summary["checkpoint"] = json.loads(revision.read_text())
    result, _ = run(["git", "-C", str(ROOT / ".runtime/ACE-Step-1.5"), "rev-parse", "HEAD"])
    if result.returncode == 0:
        summary["service_code_revision"] = result.stdout.strip()
    cases = [("drift", 54, "Warm ambient pads, a spacious and unhurried instrumental with gentle evolving textures."),
             ("focus", 76, "Soft piano with a steady gentle pulse, layered with airy strings, instrumental."),
             ("unwind", 60, "Warm fingerpicked acoustic guitar and soft sustained strings, calm instrumental.")]
    for style, tempo, prompt in cases:
        for seed in args.seeds:
            stem = f"{style}-{seed}"
            source = args.output / f"{stem}.wav"
            print(f"Generating {stem}…", flush=True)
            generated, elapsed = run([
                str(args.binary), "generate", "--engine", "ace-step", "--style", style,
                "--duration", "30", "--tempo", str(tempo), "--seed", str(seed),
                "--prompt", prompt, "-o", str(source),
            ])
            summary["sources"].append({
                "name": stem, "elapsed_s": elapsed, "success": generated.returncode == 0,
                "error": generated.stderr.strip() if generated.returncode else None,
            })
            if generated.returncode:
                print(generated.stderr, flush=True)
                continue
            for mode, center, sham in [("notch", 1000, None), ("notch", 3000, None),
                                       ("notch", 6000, None), ("notch", 10000, None),
                                       ("passthrough", 6000, None), ("sham", 6000, 1500)]:
                output = args.output / f"{stem}-{mode}-{center}.wav"
                report = args.output / f"{stem}-{mode}-{center}.json"
                command = [str(args.binary), "process", "-i", str(source), "-o", str(output),
                           "--center", str(center), "--mode", mode, "--report", str(report)]
                if sham:
                    command.extend(["--sham-center", str(sham)])
                processed, processing_s = run(command)
                record = json.loads(report.read_text()) if report.exists() else {}
                summary["checks"].append({
                    "source": stem, "mode": mode, "center_hz": center,
                    "processing_s": processing_s, "all_pass": record.get("all_pass", False),
                    "warnings": record.get("warnings", []),
                    "admitted": processed.returncode == 0 and record.get("all_pass", False) and not record.get("warnings"),
                    "error": processed.stderr.strip() if processed.returncode else None,
                    "report": report.name,
                })
            (args.output / "summary.json").write_text(json.dumps(summary, indent=2))
    summary["generated"] = sum(s["success"] for s in summary["sources"])
    summary["admitted_checks"] = sum(c["admitted"] for c in summary["checks"])
    summary["total_checks"] = len(summary["checks"])
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps({k: summary[k] for k in ["generated", "admitted_checks", "total_checks"]}))
    print(f"Results: {args.output / 'summary.json'}")
    if summary["generated"] != len(summary["sources"]):
        return 1
    return 0 if summary["admitted_checks"] == summary["total_checks"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
