#!/usr/bin/env python3
"""Record two fixed-seed Lyria RealTime streams, without enabling billing.

Credentials are read in-process from an ignored file, never from command-line
arguments. Only the experimental RealTime model is called. Preserve all returned
PCM; any clipping/fades/loudness matching belong to the separate blind-set build.
"""

from __future__ import annotations

import argparse
import array
import base64
import datetime as dt
import hashlib
import importlib.metadata
import json
import logging
import math
from pathlib import Path
import re
import sys
import time
from urllib.parse import quote
import wave

import websocket


ROOT = Path(__file__).resolve().parents[2]
MODEL = "models/lyria-realtime-exp"
ENDPOINT = (
    "wss://generativelanguage.googleapis.com/ws/"
    "google.ai.generativelanguage.v1alpha.GenerativeService.BidiGenerateMusic"
)
PROMPT = (
    "An intimate solo acoustic grand piano performance. Slow lyrical melody, "
    "warm rounded piano tone, gentle legato phrasing, sparse left-hand chords, "
    "natural expressive timing, a quiet dry studio recording. Only one acoustic "
    "grand piano. No percussion, bells, chimes, singing, synthesizers, ambient "
    "sound effects, tape hiss, vinyl crackle, or background noise."
)
SEEDS = (210731, 210732)
SOURCE_URLS = [
    "https://ai.google.dev/gemini-api/docs/realtime-music-generation",
    "https://ai.google.dev/api/live_music",
    "https://github.com/google-gemini/cookbook/blob/main/quickstarts/Get_started_LyriaRealTime.ipynb",
    "https://magenta.withgoogle.com/infinite-crate-announce",
]


def sanitize(value: object, secrets: list[str]) -> object:
    text = json.dumps(value, ensure_ascii=False)
    for secret in secrets:
        if secret:
            text = text.replace(secret, "[REDACTED]")
            text = text.replace(quote(secret, safe=""), "[REDACTED]")
    return json.loads(text)


def save_json(path: Path, value: object, secrets: list[str]) -> None:
    path.write_text(
        json.dumps(sanitize(value, secrets), indent=2, ensure_ascii=False) + "\n"
    )


def pcm_metrics(data: bytes, sample_rate: int) -> dict:
    samples = array.array("h", data)
    if sys.byteorder != "little":
        samples.byteswap()
    if not samples or len(samples) % 2:
        raise ValueError("PCM is empty or not stereo-frame aligned")
    peak = max(abs(value) for value in samples) / 32768.0
    rms = math.sqrt(sum(value * value for value in samples) / len(samples)) / 32768.0
    return {
        "frames": len(samples) // 2,
        "duration_seconds": len(samples) / (2 * sample_rate),
        "channels": 2,
        "sample_rate": sample_rate,
        "sample_format": "signed 16-bit little-endian PCM",
        "finite": True,
        "peak_dbfs": 20 * math.log10(peak) if peak else None,
        "rms_dbfs": 20 * math.log10(rms) if rms else None,
        "full_scale_samples": sum(value in (-32768, 32767) for value in samples),
        "nonzero_samples": sum(value != 0 for value in samples),
        "sha256_pcm": hashlib.sha256(data).hexdigest(),
    }


def record(seed: int, duration: float, outdir: Path, credentials: dict) -> dict:
    prefix = outdir / f"lyria-realtime-seed{seed}"
    manifest_path = prefix.with_suffix(".json")
    paths = [prefix.with_suffix(ext) for ext in (".pcm", ".wav", ".json")]
    if any(path.exists() for path in paths):
        raise FileExistsError(f"Refusing to overwrite existing seed {seed} output")
    secrets = [value for value in credentials.values() if isinstance(value, str)]
    config = {
        "seed": seed,
        "bpm": 64,
        "density": 0.2,
        "brightness": 0.3,
        "guidance": 5.0,
        "temperature": 0.9,
        "topK": 40,
        "scale": "D_MAJOR_B_MINOR",
        "muteDrums": True,
        "muteBass": False,
        "musicGenerationMode": "QUALITY",
    }
    record = {
        "schema_version": 1,
        "started_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "model": MODEL,
        "api_version": "v1alpha",
        "prompt": PROMPT,
        "prompt_weight": 1.0,
        "config": config,
        "requested_minimum_seconds": duration,
        "official_sources": SOURCE_URLS,
        "free_access_basis": "Official Google cookbook describes free preview with quota; account status reports free billing tier.",
        "billing_changes_made": False,
        "other_model_calls_made": False,
        "attempt_number": 1,
        "transport": "WebSocket over existing SOCKS5 proxy with remote DNS and verified TLS",
        "software": {
            name: importlib.metadata.version(name)
            for name in ("websocket-client", "python-socks")
        },
        "status": "started",
        "stage": "connect",
        "messages_without_audio": [],
        "chunks": [],
        "processing": "All returned PCM chunks concatenated unchanged; WAV header added. No DSP or sample selection.",
    }
    ws = None
    received = bytearray()
    sample_rate = 48000
    start = time.monotonic()
    try:
        ws = websocket.create_connection(
            ENDPOINT + "?key=" + quote(credentials["GEMINI_API_KEY"], safe=""),
            timeout=45,
            http_proxy_host="127.0.0.1",
            http_proxy_port=1080,
            proxy_type="socks5h",
            enable_multithread=False,
        )
        ws.send(json.dumps({"setup": {"model": MODEL}}))
        record["stage"] = "setup"
        setup = json.loads(ws.recv())
        record["setup_response"] = setup
        if "setupComplete" not in setup and "setup_complete" not in setup:
            raise RuntimeError("Server did not confirm setup: " + json.dumps(setup))
        ws.send(json.dumps({"clientContent": {"weightedPrompts": [{"text": PROMPT, "weight": 1.0}]}}))
        ws.send(json.dumps({"musicGenerationConfig": config}))
        ws.send(json.dumps({"playbackControl": "PLAY"}))
        record["stage"] = "receive_audio"
        deadline = time.monotonic() + 180
        while len(received) / (4 * sample_rate) < duration:
            if time.monotonic() > deadline:
                raise TimeoutError("Audio stream did not reach requested duration within 180 seconds")
            payload = ws.recv()
            if not payload:
                raise RuntimeError("Server closed the stream before requested duration")
            message = json.loads(payload)
            server_content = message.get("serverContent", message.get("server_content", {}))
            chunks = server_content.get("audioChunks", server_content.get("audio_chunks", []))
            if not chunks:
                record["messages_without_audio"].append(message)
                if "error" in message or "filteredPrompt" in message or "filtered_prompt" in message:
                    raise RuntimeError("Server rejected generation: " + json.dumps(message))
            for chunk in chunks:
                mime = chunk.get("mimeType", chunk.get("mime_type", ""))
                rate_match = re.search(r"rate=(\d+)", mime)
                next_rate = int(rate_match.group(1)) if rate_match else 48000
                if received and next_rate != sample_rate:
                    raise ValueError("Sample rate changed during stream")
                sample_rate = next_rate
                data = base64.b64decode(chunk["data"], validate=True)
                if len(data) % 4 or data.startswith(b"RIFF"):
                    raise ValueError("Unexpected non-raw or non-stereo PCM chunk")
                if mime and not any(kind in mime.lower() for kind in ("pcm", "l16")):
                    raise ValueError("Unsupported audio MIME type: " + mime)
                received.extend(data)
                record["chunks"].append({
                    "bytes": len(data),
                    "mime_type": mime,
                    "received_after_seconds": round(time.monotonic() - start, 3),
                    "source_metadata": chunk.get("sourceMetadata", chunk.get("source_metadata")),
                })
        ws.send(json.dumps({"playbackControl": "STOP"}))
        record["status"] = "completed"
        record["stage"] = "complete"
    except Exception as exc:
        record["status"] = "failed"
        record["failure"] = {"type": type(exc).__name__, "message": str(exc)}
        if getattr(exc, "status_code", None) is not None:
            record["failure"]["http_status"] = exc.status_code
    finally:
        if ws is not None:
            try:
                ws.close()
            except Exception:
                pass
        record["elapsed_seconds"] = round(time.monotonic() - start, 3)
        if received:
            prefix.with_suffix(".pcm").write_bytes(received)
            with wave.open(str(prefix.with_suffix(".wav")), "wb") as wav:
                wav.setnchannels(2)
                wav.setsampwidth(2)
                wav.setframerate(sample_rate)
                wav.writeframes(received)
            record["audio"] = pcm_metrics(bytes(received), sample_rate)
            record["audio"]["wav_path"] = str(prefix.with_suffix(".wav").relative_to(ROOT))
            record["audio"]["sha256_wav"] = hashlib.sha256(prefix.with_suffix(".wav").read_bytes()).hexdigest()
        save_json(manifest_path, record, secrets)
    return sanitize(record, secrets)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--duration", type=float, default=40.0)
    parser.add_argument("--output-dir", type=Path, default=ROOT / "artifacts/audio/blind-20261021/lyria-realtime")
    args = parser.parse_args()
    if not 35 <= args.duration <= 60:
        parser.error("duration must be between 35 and 60 seconds")
    logging.disable(logging.CRITICAL)
    websocket.enableTrace(False)
    credentials = json.loads((ROOT / ".runtime/music-access/credentials.json").read_text())
    status = json.loads((ROOT / ".runtime/music-access/status.json").read_text())
    if status.get("providers", {}).get("google_ai_studio", {}).get("billing_tier") != "free":
        raise SystemExit("This runner requires the recorded free-tier account; refusing an unverified billing tier.")
    if not credentials.get("GEMINI_API_KEY"):
        raise SystemExit("GEMINI_API_KEY is not configured")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for seed in SEEDS:
        result = record(seed, args.duration, args.output_dir, credentials)
        print(json.dumps({key: result.get(key) for key in ("status", "config", "stage", "audio", "failure")}), flush=True)
        if result["status"] != "completed":
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
