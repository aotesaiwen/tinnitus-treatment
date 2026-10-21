#!/usr/bin/env python3
"""Reproducible Aria continuations of an original eight-bar piano seed.

Run with .runtime/aria-venv/bin/python. The runtime imports the pinned official
Aria checkout; the output contains MIDI and provenance, never replacement audio.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import io
import json
from pathlib import Path
import random
import subprocess
import time

import mido


ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "artifacts/audio/blind-20261021/aria"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_events(path: Path, events: list[tuple[float, mido.Message]], end_s: float, reuse_identical: bool = False):
    midi = mido.MidiFile(type=0, ticks_per_beat=500)
    track = mido.MidiTrack()
    midi.tracks.append(track)
    track.append(mido.MetaMessage("set_tempo", tempo=500000, time=0))
    track.append(mido.MetaMessage("time_signature", numerator=4, denominator=4, time=0))
    last = 0
    for seconds, message in sorted(events, key=lambda item: (item[0], item[1].type == "note_on")):
        tick = round(max(0.0, seconds) * 1000)
        track.append(message.copy(time=tick - last))
        last = tick
    track.append(mido.MetaMessage("end_of_track", time=max(0, round(end_s * 1000) - last)))
    buffer = io.BytesIO()
    midi.save(file=buffer)
    payload = buffer.getvalue()
    if reuse_identical and path.exists() and path.read_bytes() == payload:
        return
    with path.open("xb") as handle:
        handle.write(payload)


def make_seed() -> dict:
    """Eight original bars: Dmaj7, Bm7, Gmaj7, Asus4/A; modest rubato.

    Both hands remain on one acoustic-piano track. The melody is explicitly
    authored for this experiment; it is not a copyrighted prompt or a model demo.
    """
    OUT.mkdir(parents=True, exist_ok=True)
    rng = random.Random(20261021)
    # Beat lengths slow at phrase endings; no randomized note selection.
    beat_lengths = [0.91] * 3 + [1.00] + [0.92] * 3 + [1.00]
    beat_lengths += [0.92] * 3 + [1.01] + [0.94, 0.95, 0.99, 1.09]
    beat_lengths += [0.91] * 3 + [0.98] + [0.92] * 3 + [1.00]
    beat_lengths += [0.94] * 3 + [1.04] + [0.95, 0.97, 1.04, 1.12]
    beat_starts = [0.0]
    for length in beat_lengths:
        beat_starts.append(beat_starts[-1] + length)

    def beat_time(beat: float) -> float:
        integer = min(int(beat), 31)
        return beat_starts[integer] + (beat - integer) * beat_lengths[integer]

    chords = [
        (38, 57, 61, 66), (35, 54, 57, 62),
        (31, 54, 59, 62), (33, 52, 57, 62),
        (38, 57, 61, 66), (35, 54, 57, 62),
        (31, 54, 59, 62), (33, 52, 57, 61),
    ]
    # (offset in bar, pitch, length in beats, velocity), shaped as two phrases.
    melody = [
        [(0.0, 66, 1.35, 65), (1.5, 69, 0.65, 69), (2.5, 73, 1.2, 72)],
        [(0.0, 71, 1.4, 68), (1.5, 69, 0.65, 65), (2.5, 66, 1.1, 61)],
        [(0.0, 67, 0.8, 64), (1.0, 69, 0.8, 67), (2.0, 71, 1.5, 69)],
        [(0.0, 69, 1.1, 63), (1.5, 66, 0.8, 60), (2.5, 64, 1.0, 56)],
        [(0.0, 66, 1.35, 65), (1.5, 69, 0.65, 69), (2.5, 73, 1.0, 73)],
        [(0.0, 74, 0.8, 74), (1.0, 73, 0.8, 70), (2.0, 71, 1.35, 67)],
        [(0.0, 69, 1.3, 66), (1.5, 67, 0.75, 62), (2.5, 66, 1.15, 60)],
        [(0.0, 64, 1.4, 59), (1.5, 61, 0.75, 55), (2.5, 64, 1.1, 59)],
    ]
    events = [(0.0, mido.Message("program_change", channel=0, program=0))]
    notes = []

    def note(beat: float, pitch: int, length: float, velocity: int, hand: str):
        onset = max(0, beat_time(beat) + rng.uniform(-0.008, 0.011))
        # Duration carries legato without sustaining every chord across bar lines.
        end = beat_time(min(32.0, beat + length))
        velocity = max(1, velocity + rng.randint(-2, 2))
        events.extend([
            (onset, mido.Message("note_on", channel=0, note=pitch, velocity=velocity)),
            (end, mido.Message("note_off", channel=0, note=pitch, velocity=0)),
        ])
        notes.append({"onset_s": onset, "end_s": end, "pitch": pitch, "velocity": velocity, "hand": hand})

    for bar, chord in enumerate(chords):
        base = bar * 4.0
        bass, lower, middle, upper = chord
        note(base, bass, 2.9, 43, "left")
        note(base + 0.5, lower, 1.25, 42, "left")
        note(base + 1.0, middle, 1.2, 45, "left")
        note(base + 2.0, upper, 1.1, 46, "left")
        note(base + 3.0, middle, 0.8, 41, "left")
        for offset, pitch, length, velocity in melody[bar]:
            note(base + offset, pitch, length, velocity, "right")

    path = OUT / "original-eight-bar-seed.mid"
    write_events(path, events, beat_starts[-1], reuse_identical=True)
    result = {
        "path": str(path), "duration_s": beat_starts[-1], "note_count": len(notes),
        "key": "D major", "meter": "4/4", "bars": 8,
        "approximate_bpm": 60 / (beat_starts[-1] / 32),
        "origin": "Original notes authored in this script for this comparison; not a published song or model demo.",
        "notes": notes, "sha256": sha256(path),
    }
    seed_json = OUT / "seed.json"
    payload = json.dumps(result, indent=2) + "\n"
    if not (seed_json.exists() and seed_json.read_text() == payload):
        with seed_json.open("x") as handle:
            handle.write(payload)
    return result


def crop_generated(midi_dict, start_ms: int, duration_ms: int, path: Path) -> dict:
    # The official tokenizer uses one millisecond per tick on detokenization.
    # Crop by note onset so none of the authored input notes enter the blind clip.
    end_ms = start_ms + duration_ms
    events = [(0.0, mido.Message("program_change", channel=0, program=0))]
    included = []
    for msg in midi_dict.note_msgs:
        data = msg["data"]
        if start_ms <= data["start"] < end_ms:
            onset = (data["start"] - start_ms) / 1000
            end = (min(data["end"], end_ms) - start_ms) / 1000
            if end <= onset:
                continue
            channel = msg["channel"]
            events.extend([
                (onset, mido.Message("note_on", channel=channel, note=data["pitch"], velocity=data["velocity"])),
                (end, mido.Message("note_off", channel=channel, note=data["pitch"], velocity=0)),
            ])
            included.append(data)
    if not included:
        raise RuntimeError("The model produced no notes after the original prompt.")
    last_end = max(msg["end"] for msg in included)
    actual_s = min(duration_ms, last_end - start_ms) / 1000
    write_events(path, events, actual_s)
    return {
        "path": str(path), "duration_s": actual_s, "note_count": len(included),
        "crop_start_s": start_ms / 1000, "requested_duration_s": duration_ms / 1000,
        "pitch_range": [min(n["pitch"] for n in included), max(n["pitch"] for n in included)],
        "velocity_range": [min(n["velocity"] for n in included), max(n["velocity"] for n in included)],
        "sha256": sha256(path),
        "processing": "Crop only; original input excluded; note pitches, timings and velocities unchanged; acoustic piano program 0.",
    }


def generate(seed_metadata: dict, seeds: list[int], max_new_tokens: int):
    import numpy as np
    import torch
    from safetensors.torch import load_file
    from aria.config import load_model_config
    from aria.model import ModelConfig
    from aria.inference import get_inference_prompt
    from aria.inference.model_cuda import TransformerLM
    from aria.inference.sample_cuda import sample_batch
    from ariautils.midi import MidiDict
    from ariautils.tokenizer import AbsTokenizer

    checkpoint = ROOT / ".runtime/aria-checkpoint/model-gen.safetensors"
    provenance = json.loads((checkpoint.parent / "provenance.json").read_text())
    torch.set_num_threads(8)
    torch.cuda.set_per_process_memory_fraction(16 * 1024**3 / torch.cuda.get_device_properties(0).total_memory)
    tokenizer = AbsTokenizer()
    model_config = ModelConfig(**load_model_config(name="medium"))
    model_config.set_vocab_size(tokenizer.vocab_size)
    model = TransformerLM(model_config)
    state_dict = load_file(str(checkpoint))
    missing, unexpected = model.load_state_dict(state_dict, strict=False)
    if missing:
        raise RuntimeError(f"Missing model parameters: {missing}")
    del state_dict
    model = model.to(dtype=torch.bfloat16).cuda().eval()
    prompt = get_inference_prompt(
        MidiDict.from_midi(seed_metadata["path"]), tokenizer,
        prompt_len_ms=round(seed_metadata["duration_s"] * 1000),
    )
    provenance.update({
        "code_revision": subprocess.check_output(["git", "-C", str(ROOT / ".runtime/aria"), "rev-parse", "HEAD"], text=True).strip(),
        "tokenizer_revision": subprocess.check_output(["git", "-C", str(ROOT / ".runtime/aria-utils"), "rev-parse", "HEAD"], text=True).strip(),
        "checkpoint_file": str(checkpoint), "checkpoint_sha256": sha256(checkpoint),
        "source_model_card": "https://huggingface.co/loubb/aria-medium-base",
        "source_code": "https://github.com/EleutherAI/aria",
        "license": "Apache-2.0 (official model card and source repository)",
        "checkpoint_resolution": "The source README's loubb/aria-medium-gen endpoint returned HTTP 401. The official author's base model card links the same generative variant as model-gen.safetensors in loubb/aria-medium-base; that public file is used at the recorded revision.",
        "checkpoint_variant": "model-gen.safetensors (official generative finetune)",
        "pytorch": torch.__version__, "gpu": torch.cuda.get_device_name(0),
        "dtype": "bfloat16", "allocator_limit_gib": 16,
        "ignored_checkpoint_keys": list(unexpected), "prompt_tokens": len(prompt),
        "temperature": 0.90, "min_p": 0.035, "max_new_tokens": max_new_tokens,
        "force_end": False, "compile": False, "seed": seed_metadata,
        "samples": [],
        "limitations": [
            "Aria continues symbolic piano; no text instruction following is claimed.",
            "MIDI notes are generated by the model; piano timbre must be rendered separately.",
            "Blind excerpts exclude the original input and retain generated note timings and velocities.",
            "No human listening or musical-quality claim is made by this generation script.",
        ],
    })
    report_path = OUT / "generation.json"
    with report_path.open("x") as handle:
        handle.write(json.dumps(provenance, indent=2) + "\n")
    for seed in seeds:
        random.seed(seed)
        np.random.seed(seed)
        torch.manual_seed(seed)
        torch.cuda.manual_seed_all(seed)
        torch.cuda.reset_peak_memory_stats()
        torch.cuda.synchronize()
        started = time.monotonic()
        result = sample_batch(
            model=model, tokenizer=tokenizer, prompt=copy.deepcopy(prompt),
            num_variations=1, max_new_tokens=max_new_tokens, temp=0.90,
            min_p=0.035, top_p=None, force_end=False, compile=False,
        )[0]
        torch.cuda.synchronize()
        elapsed = time.monotonic() - started
        full = tokenizer.detokenize(result)
        full_path = OUT / f"aria-seed-{seed}-full.mid"
        with full_path.open("xb") as handle:
            full.to_midi().save(file=handle)
        clip_path = OUT / f"aria-seed-{seed}-continuation.mid"
        crop = crop_generated(full, round(seed_metadata["duration_s"] * 1000), 60000, clip_path)
        record = {
            "seed": seed, "generation_seconds": elapsed,
            "gpu_peak_allocated_gib": torch.cuda.max_memory_allocated() / 1024**3,
            "full_midi": str(full_path), "full_sha256": sha256(full_path),
            "full_duration_s": max(m["data"]["end"] for m in full.note_msgs) / 1000,
            "generated_tokens": len(result) - len(prompt), "clip": crop,
        }
        provenance["samples"].append(record)
        report_path.write_text(json.dumps(provenance, indent=2) + "\n")
        print(json.dumps(record), flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--seeds", type=int, nargs="+", default=[21021, 21022])
    parser.add_argument("--max-new-tokens", type=int, default=2048)
    args = parser.parse_args()
    if not args.prepare_only:
        existing = [path for path in [OUT / "generation.json", *OUT.glob("aria-seed-*.mid")] if path.exists()]
        if existing:
            raise FileExistsError("Refusing to overwrite an existing Aria run: " + ", ".join(str(path) for path in existing))
    seed = make_seed()
    print(json.dumps({key: value for key, value in seed.items() if key != "notes"}), flush=True)
    if not args.prepare_only:
        generate(seed, args.seeds, args.max_new_tokens)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        import traceback

        OUT.mkdir(parents=True, exist_ok=True)
        error_path = OUT / "generation-error.json"
        if not error_path.exists() and not isinstance(error, FileExistsError):
            with error_path.open("x") as handle:
                handle.write(json.dumps({
                    "error_type": type(error).__name__, "message": str(error),
                    "traceback": traceback.format_exc(),
                }, indent=2) + "\n")
        raise
