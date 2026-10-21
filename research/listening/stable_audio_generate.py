#!/usr/bin/env python3
"""Generate two fixed-seed solo-piano clips from a verified Stable Audio model."""
from __future__ import annotations

import argparse
from importlib.metadata import version
import json
import os
from pathlib import Path
import subprocess
import time

os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"
os.environ["ENABLE_TORCH_COMPILE"] = "0"

import numpy as np
import soundfile as sf
import torch

from stable_audio_setup import DESTINATION, REVISIONS, digest

ROOT = Path(__file__).resolve().parents[2]
CODE_REVISION = "77e0398c26f7d22d801ec1bb036307e8b231f741"
PROMPT = (
    "TrackType: Instrument, Format: Solo, Instruments: Acoustic Grand Piano, "
    "Genre: Contemporary Classical, VocalType: Instrumental. "
    "A quiet, intimate solo acoustic grand piano piece at 64 BPM in D major. "
    "A clear, lyrical right-hand melody over sparse, gently flowing left-hand broken chords. "
    "Natural phrasing with breathing space, soft rounded notes, warm lower and middle registers, "
    "restrained dynamics. Clean close-miked studio piano with a short natural room decay. "
    "One acoustic piano alone throughout, without accompaniment."
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True, choices=REVISIONS)
    parser.add_argument("--seeds", nargs="+", type=int, default=[210731, 210732])
    parser.add_argument("--duration", type=int, default=60)
    args = parser.parse_args()
    if not 35 <= args.duration <= 120:
        raise ValueError("Use a full 35–120 second source for the 30-second blind excerpt")
    if len(set(args.seeds)) != len(args.seeds):
        raise ValueError("Seeds must be unique")
    revision = subprocess.check_output(["git", "-C", str(ROOT / ".runtime/stable-audio-3"),
                                       "rev-parse", "HEAD"], text=True).strip()
    if revision != CODE_REVISION:
        raise ValueError("Unexpected Stable Audio source revision")
    checkpoint = DESTINATION / args.model
    verified = json.loads((checkpoint / "verified.json").read_text())
    if verified["revision"] != REVISIONS[args.model]:
        raise ValueError("Unexpected checkpoint revision")
    for item in verified["files"]:
        path = checkpoint / item["path"]
        if path.stat().st_size != item["bytes"] or digest(path) != item["sha256"]:
            raise ValueError(f"Checkpoint integrity failure: {item['path']}")
    output = ROOT / "artifacts/audio/blind-20261021/stable-audio" / args.model
    outputs = [output / f"piano-{seed}-{args.duration}s-raw.wav" for seed in args.seeds]
    if any(path.exists() for path in outputs) or (output / "generation.json").exists():
        raise FileExistsError("Refusing to overwrite an existing generation")
    if not torch.cuda.is_available():
        raise RuntimeError("This recorded comparison uses a CUDA GPU")
    torch.set_num_threads(8)
    torch.cuda.set_per_process_memory_fraction(min(1.0, 40 * 2**30 / torch.cuda.get_device_properties(0).total_memory))

    import stable_audio_3.loading_utils as loading
    from stable_audio_3 import StableAudioModel
    from stable_audio_3.models import transformer

    strict_loads = []
    def checked_load(model, state_dict):
        mapped = loading.remap_state_dict_keys(state_dict, model.state_dict())
        model.load_state_dict(mapped, strict=True)
        strict_loads.append({"loaded_tensors": len(mapped), "strict": True})
    # The upstream permissive loader can leave random parameters on a mismatch.
    # This hook requires every tensor to match; it does not alter the checkpoint.
    loading.copy_state_dict = checked_load
    config = json.loads((checkpoint / "model_config.json").read_text())
    for condition in config["model"]["conditioning"]["configs"]:
        if condition["type"] == "t5gemma":
            condition["config"]["model_path"] = str(checkpoint / "t5gemma-b-b-ul2")
            condition["config"].pop("repo_id", None)
            condition["config"].pop("subfolder", None)
    started = time.monotonic()
    base = loading.load_diffusion_cond(config, str(checkpoint / "model.safetensors"), device="cuda", model_half=True)
    base.use_lora = False
    base.lora_names = []
    model = StableAudioModel(base, config, "cuda", True)
    load_seconds = time.monotonic() - started
    output.mkdir(parents=True, exist_ok=True)
    record = {
        "model": args.model, "code_revision": revision, "checkpoint_revision": verified["revision"],
        "checkpoint_files": verified["files"], "prompt": PROMPT, "duration_seconds": args.duration,
        "steps": 8, "cfg_scale": 1.0, "negative_prompt": None, "model_dtype": "float16",
        "chunked_decode": False, "load_seconds": load_seconds, "strict_loads": strict_loads,
        "gpu": torch.cuda.get_device_name(), "memory_limit_GiB": 40,
        "flash_attention_installed": transformer.flash_attn_func is not None,
        "attention_note": "Upstream attention implementation with its built-in flex/SDPA fallbacks",
        "software": {name: version(name) for name in ("torch", "torchaudio", "transformers", "numpy", "safetensors")},
        "samples": [],
    }
    print(json.dumps({"model": args.model, "loaded_seconds": load_seconds, "strict_loads": strict_loads}), flush=True)
    for seed, destination in zip(args.seeds, outputs):
        torch.cuda.reset_peak_memory_stats()
        started = time.monotonic()
        result = model.generate(prompt=PROMPT, duration=args.duration, steps=8, cfg_scale=1,
                                seed=seed, batch_size=1, chunked_decode=False)
        torch.cuda.synchronize()
        elapsed = time.monotonic() - started
        audio = result[0].float().cpu().numpy().T
        if audio.shape != (44100 * args.duration, 2) or not np.isfinite(audio).all():
            raise ValueError(f"Invalid generated audio shape or values: {audio.shape}")
        if np.max(np.abs(audio)) < 1e-4:
            raise ValueError("Generated audio is silent")
        sf.write(destination, audio, 44100, subtype="FLOAT")
        sample = {"seed": seed, "path": str(destination.relative_to(ROOT)),
                  "duration_seconds": len(audio) / 44100, "sample_rate": 44100, "channels": 2,
                  "generation_seconds": elapsed, "peak_gpu_GiB": torch.cuda.max_memory_allocated() / 2**30,
                  "peak": float(np.max(np.abs(audio))), "rms": float(np.sqrt(np.mean(audio.astype(np.float64)**2))),
                  "sha256": digest(destination)}
        record["samples"].append(sample)
        (output / "generation.json").write_text(json.dumps(record, indent=2) + "\n")
        print(json.dumps(sample), flush=True)
        del result


if __name__ == "__main__":
    main()
