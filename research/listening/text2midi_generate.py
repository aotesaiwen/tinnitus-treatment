#!/usr/bin/env python3
"""Generate auditable MIDI using the official Text2midi implementation.

Run with .runtime/text2midi-env/bin/python. Runtime dependencies are isolated;
the existing ACE-Step environment is read-only and supplies cached PyTorch.
The original MIDI is retained without instrument, timing, or note edits.
"""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import pickle
import random
import subprocess
import sys
import time
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / ".runtime/text2midi"
WEIGHTS = ROOT / ".runtime/text2midi-weights"
ENCODER = ROOT / ".runtime/text2midi-flan-t5-base"
DEFAULT_OUTPUT = ROOT / "artifacts/audio/blind-20261021/text2midi"
PROMPTS = [
    "A gentle expressive solo acoustic grand piano piece in C major, 4/4 time, "
    "at a slow tempo of 64 beats per minute. A sparse lyrical right-hand melody "
    "floats above soft left-hand broken chords and arpeggios. Calm, intimate, "
    "warm and reflective, with quiet dynamics and breathing space between "
    "phrases. Piano is the only instrument. No drums, no percussion, no other "
    "instruments. Approximately one minute long.",
    "A quiet and tender solo acoustic grand piano composition in A minor, "
    "4/4 time, at 68 beats per minute. A simple singing melody with expressive "
    "phrasing and gentle arpeggiated accompaniment. Soft dynamics, spacious "
    "phrases, a peaceful reflective mood and a resolved ending. Piano alone, "
    "without drums, percussion or any other instrument. About one minute long.",
]


def write_json(path: Path, data: object) -> None:
    path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")


def midi_summary(path: Path) -> dict:
    import mido

    midi = mido.MidiFile(path)
    events = list(midi)
    duration = sum(event.time for event in events)
    notes = [event for event in events if event.type == "note_on" and event.velocity > 0]
    notes_per_channel = Counter(event.channel for event in notes)
    return {
        "duration_s": duration,
        "note_count": len(notes),
        "percussion_note_count": notes_per_channel[9],
        "pitched_note_count": sum(count for channel, count in notes_per_channel.items() if channel != 9),
        "notes_per_channel": dict(notes_per_channel),
        "pitch_min": min((event.note for event in notes), default=None),
        "pitch_max": max((event.note for event in notes), default=None),
        "velocities": sorted({event.velocity for event in notes}),
        "program_changes": [
            {"channel": event.channel, "program_zero_based": event.program}
            for event in events if event.type == "program_change"
        ],
        "channels_with_notes": sorted({event.channel for event in notes}),
        "tempo_bpm": [mido.tempo2bpm(event.tempo) for event in events if event.type == "set_tempo"],
        "tracks": len(midi.tracks),
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--max-tokens", type=int, default=2000)
    parser.add_argument("--temperature", type=float, default=0.9)
    parser.add_argument("--seeds", type=int, nargs="+", default=[210641, 210682])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    for index, seed in enumerate(args.seeds):
        existing = args.output / f"text2midi-{index + 1}-seed{seed}.mid"
        if existing.exists():
            raise FileExistsError(f"Refusing to overwrite generated sample or its manifest: {existing}")

    import numpy as np
    import torch
    from transformers import T5Config, T5EncoderModel, T5Tokenizer

    if not torch.cuda.is_available():
        raise RuntimeError("This benchmark requires the allocated CUDA GPU.")
    torch.cuda.set_per_process_memory_fraction(
        min(1.0, 20 * 1024**3 / torch.cuda.get_device_properties(0).total_memory)
    )
    torch.set_num_threads(8)
    sys.path.insert(0, str(SOURCE))
    from model.transformer_model import Transformer

    manifest = {
        "model": "amaai-lab/text2midi",
        "model_revision": json.loads((WEIGHTS / "revision.json").read_text())["revision"],
        "checkpoint_verification": json.loads((WEIGHTS / "revision.json").read_text()),
        "source_url": "https://github.com/AMAAI-Lab/Text2midi",
        "source_revision": subprocess.check_output(["git", "-C", str(SOURCE), "rev-parse", "HEAD"], text=True).strip(),
        "encoder": json.loads((ENCODER / "revision.json").read_text()),
        "architecture_arguments": ["vocab_size", 768, 8, 2048, 18, 1024, False, 8],
        "implementation": "Official Transformer.generate; torch.inference_mode; float32; no sampling changes",
        "encoder_initialization": "Architecture from pinned FLAN-T5 config; complete encoder weights required in the Text2midi checkpoint via strict state-dict load",
        "dependency_versions": {},
        "midi_transformations": [],
        "generation": {"max_tokens": args.max_tokens, "temperature": args.temperature},
        "samples": [],
    }
    from importlib.metadata import version
    for package in ["torch", "transformers", "miditok", "symusic", "tokenizers"]:
        manifest["dependency_versions"][package] = version(package)
    write_json(args.output / "manifest.json", manifest)

    with (WEIGHTS / "vocab_remi.pkl").open("rb") as source:
        midi_tokenizer = pickle.load(source)
    # The complete official checkpoint contains the encoder as well as decoder.
    # Instantiate its exact pinned architecture without redundantly downloading
    # FLAN-T5's decoder weights. strict=True below rejects missing parameters.
    encoder_config = T5Config.from_pretrained(str(ENCODER))
    with patch.object(T5EncoderModel, "from_pretrained", side_effect=lambda *_a, **_kw: T5EncoderModel(encoder_config)):
        model = Transformer(len(midi_tokenizer), 768, 8, 2048, 18, 1024, False, 8, device="cuda")
    state = torch.load(WEIGHTS / "pytorch_model.bin", map_location="cpu", weights_only=True)
    model.load_state_dict(state, strict=True)
    del state
    model.eval()
    text_tokenizer = T5Tokenizer.from_pretrained(str(ENCODER))
    print("Official Text2midi model loaded.", flush=True)

    for index, seed in enumerate(args.seeds):
        name = f"text2midi-{index + 1}-seed{seed}"
        midi_path = args.output / f"{name}.mid"
        if midi_path.exists():
            raise FileExistsError(f"Refusing to overwrite generated sample: {midi_path}")
        prompt = PROMPTS[index % len(PROMPTS)]
        random.seed(seed)
        np.random.seed(seed)
        torch.manual_seed(seed)
        torch.cuda.manual_seed_all(seed)
        inputs = text_tokenizer(prompt, return_tensors="pt", padding=True, truncation=True).to("cuda")
        entry = {"id": name, "seed": seed, "prompt": prompt, "status": "running"}
        manifest["samples"].append(entry)
        write_json(args.output / "manifest.json", manifest)
        print(json.dumps(entry), flush=True)
        start = time.perf_counter()
        with torch.inference_mode():
            output = model.generate(inputs.input_ids, inputs.attention_mask, max_len=args.max_tokens, temperature=args.temperature)
        tokens = output[0].tolist()
        torch.cuda.synchronize()
        entry["generation_seconds"] = time.perf_counter() - start
        write_json(args.output / f"{name}.tokens.json", tokens)
        midi = midi_tokenizer.decode(tokens)
        midi.dump_midi(str(midi_path))
        analysis = midi_summary(midi_path)
        active_channels = set(analysis["channels_with_notes"])
        solo_piano = analysis["percussion_note_count"] == 0 and all(
            change["program_zero_based"] == 0
            for change in analysis["program_changes"]
            if change["channel"] in active_channels
        )
        entry.update({"status": "complete", "midi": str(midi_path), "token_count": len(tokens), "analysis": analysis, "requested_solo_piano_satisfied": solo_piano, "midi_transformations": []})
        write_json(args.output / "manifest.json", manifest)
        print(json.dumps(entry), flush=True)


if __name__ == "__main__":
    main()
