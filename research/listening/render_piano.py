#!/usr/bin/env python3
"""Render MIDI through the real Salamander piano samples with local sfizz.

This wrapper does not generate or edit MIDI notes. Mechanical key/pedal samples
are omitted from a derived SFZ, while piano note and string-release samples stay
intact. Keep the original SFZ, sample archive, and render manifests for provenance.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import struct
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
PIANO = ROOT / ".runtime/piano/SalamanderGrandPianoV3_44.1khz16bit"
RENDERER = ROOT / ".runtime/sfizz/root/usr/bin/sfizz_render"
ARCHIVE_SHA256 = "fe595c7722b70860e6377f82948d1c0cfcf27ecebf500dc79534408a78a62892"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def quiet_instrument(source: Path) -> tuple[Path, dict]:
    """Derive the same documented mechanical-noise setting for every model."""
    text = source.read_text()
    marker = "//HammerNoise"
    if text.count(marker) != 1 or "//pedalAction" not in text.split(marker)[1]:
        raise ValueError("Expected the original Salamander V3 SFZ mapping.")
    musical, mechanical = text.split(marker, 1)
    # The original instrument places all key-release and pedal-action noises at
    # its end. Refuse unexpected changes instead of silently dropping notes.
    removed = re.findall(r"sample=([^\s]+)", mechanical)
    if len(removed) != 92 or any(
        not re.fullmatch(r"(?:rel\d+|pedal[DU][12])\.wav", value.replace("\\", "/").split("/")[-1])
        for value in removed
    ):
        raise ValueError("Unexpected mechanical sample section; inspect the SFZ.")
    for sample in re.findall(r"sample=([^\s]+)", musical):
        if not (source.parent / sample.replace("\\", "/")).is_file():
            raise FileNotFoundError(f"Missing original piano sample: {sample}")
    target = source.with_name("SalamanderGrandPianoV3-quiet.sfz")
    content = (
        "// Derived from Alexander Holm's Salamander Grand Piano V3 (CC BY 3.0).\n"
        "// Mechanical key/pedal noises omitted; note and resonance samples unchanged.\n"
        + musical.replace("\\", "/")
    )
    if not target.exists() or target.read_text() != content:
        with tempfile.NamedTemporaryFile(mode="w", dir=source.parent, suffix=".sfz.pending", delete=False) as temporary:
            temporary.write(content)
            pending = Path(temporary.name)
        pending.replace(target)
    return target, {
        "name": "Salamander Grand Piano V3",
        "author": "Alexander Holm",
        "license": "CC BY 3.0",
        "license_url": "https://creativecommons.org/licenses/by/3.0/",
        "source_url": "https://archive.org/details/SalamanderGrandPianoV3",
        "sample_archive_sha256": ARCHIVE_SHA256,
        "original_sfz_sha256": sha256(source),
        "rendered_sfz_sha256": sha256(target),
        "mechanical_noise_regions_omitted": len(removed),
        "note_and_string_release_samples_modified": False,
        "sample_format": "44100 Hz, 16-bit stereo PCM",
    }


def midi_stats(path: Path, *, require_piano_range: bool = True) -> dict:
    """Keep original channel/program information visible in the private record."""
    data = path.read_bytes()
    if data[:4] != b"MThd" or len(data) < 14:
        raise ValueError("Input must be a complete standard MIDI file.")
    header_size = int.from_bytes(data[4:8], "big")
    midi_format, tracks, division = struct.unpack_from(">HHH", data, 8)
    if header_size < 6 or midi_format not in (0, 1) or division & 0x8000:
        raise ValueError("Expected synchronous format 0/1 MIDI with beat-based timing.")
    offset = 8 + header_size
    notes, velocities, channels, programs = [], [], set(), set()
    pitched_notes = []
    channel_notes = {}
    drum_notes = 0
    for _ in range(tracks):
        if data[offset:offset + 4] != b"MTrk":
            raise ValueError("Missing MIDI track chunk.")
        size = int.from_bytes(data[offset + 4:offset + 8], "big")
        track = data[offset + 8:offset + 8 + size]
        if len(track) != size:
            raise ValueError("Truncated MIDI track.")
        offset += size + 8
        cursor, running = 0, None

        def variable():
            nonlocal cursor
            value = 0
            for _ in range(4):
                byte = track[cursor]
                cursor += 1
                value = (value << 7) | (byte & 0x7F)
                if not byte & 0x80:
                    return value
            raise ValueError("Invalid MIDI variable-length value.")

        while cursor < len(track):
            variable()
            status = track[cursor]
            if status & 0x80:
                cursor += 1
            elif running is not None:
                status = running
            else:
                raise ValueError("MIDI running status is missing.")
            if status in (0xF0, 0xF7, 0xFF):
                running = None
                if status == 0xFF:
                    cursor += 1  # Meta-event kind.
                length = variable()
                cursor += length
                if cursor > len(track):
                    raise ValueError("Truncated MIDI system/meta event.")
                continue
            if not 0x80 <= status <= 0xEF:
                raise ValueError("Unsupported MIDI system event.")
            running = status
            kind, channel = status & 0xF0, status & 0x0F
            length = 1 if kind in (0xC0, 0xD0) else 2
            message = track[cursor:cursor + length]
            if len(message) != length or any(value > 127 for value in message):
                raise ValueError("Invalid MIDI channel message.")
            cursor += length
            if kind == 0xC0:
                programs.add((channel, message[0]))
            if kind == 0x90 and message[1]:
                notes.append(message[0])
                velocities.append(message[1])
                channels.add(channel)
                drum_notes += channel == 9
                channel_notes[channel] = channel_notes.get(channel, 0) + 1
                if channel != 9:
                    pitched_notes.append(message[0])
    if not notes:
        raise ValueError("MIDI has no played notes.")
    if require_piano_range and (drum_notes or any(not 21 <= note <= 108 for note in notes)):
        raise ValueError("This piano renderer requires pitched piano-range notes; inspect drums/out-of-range notes separately.")
    return {
        "format": midi_format, "tracks": tracks, "ticks_per_beat": division,
        "note_on_count": len(notes), "pitch_range": [min(notes), max(notes)],
        "pitched_note_count": len(pitched_notes), "percussion_note_count": drum_notes,
        "pitched_notes_outside_piano_range": sum(not 21 <= note <= 108 for note in pitched_notes),
        "note_counts_by_channel_zero_based": {str(key): value for key, value in sorted(channel_notes.items())},
        "velocity_range": [min(velocities), max(velocities)],
        "channels_zero_based": sorted(channels),
        "program_changes_zero_based": [
            {"channel": channel, "program": program} for channel, program in sorted(programs)
        ],
        "render_mapping": "Every pitched track uses the same Salamander piano; original MIDI programs are retained here and in the unchanged MIDI.",
    }


def wav_stats(path: Path) -> dict:
    """Read PCM/float WAV metrics without altering the sampler's output."""
    with path.open("rb") as stream:
        header = stream.read(12)
        if header[:4] != b"RIFF" or header[8:] != b"WAVE":
            raise ValueError("Renderer did not produce a RIFF WAV.")
        format_info = None
        data_offset = data_size = None
        while chunk := stream.read(8):
            if len(chunk) != 8:
                raise ValueError("Truncated WAV chunk.")
            tag, size = struct.unpack("<4sI", chunk)
            if tag == b"fmt ":
                body = stream.read(size)
                kind, channels, rate, _, align, bits = struct.unpack_from("<HHIIHH", body)
                if kind == 0xFFFE and len(body) >= 40:
                    kind = struct.unpack_from("<H", body, 24)[0]
                format_info = kind, channels, rate, align, bits
            elif tag == b"data":
                data_offset, data_size = stream.tell(), size
                stream.seek(size, 1)
            else:
                stream.seek(size, 1)
            if size % 2:
                stream.seek(1, 1)
        if format_info is None or data_offset is None or data_size is None:
            raise ValueError("WAV is missing format or samples.")
        kind, channels, rate, align, bits = format_info
        if channels != 2 or rate != 44100 or data_size % align:
            raise ValueError("Expected a complete 44.1 kHz stereo render.")
        stream.seek(data_offset)
        remaining = data_size
        peak = energy = 0.0
        samples = 0
        while remaining:
            block = stream.read(min(align * 8192, remaining))
            if not block:
                raise ValueError("Truncated audio samples.")
            remaining -= len(block)
            if kind == 3 and bits in (32, 64):
                values = (v[0] for v in struct.iter_unpack("<f" if bits == 32 else "<d", block))
            elif kind == 1 and bits in (16, 24, 32):
                width = bits // 8
                values = (
                    int.from_bytes(block[i:i + width], "little", signed=True) / (2 ** (bits - 1))
                    for i in range(0, len(block), width)
                )
            else:
                raise ValueError(f"Unsupported WAV format {kind}/{bits}.")
            for value in values:
                if not math.isfinite(value):
                    raise ValueError("Renderer produced non-finite audio.")
                peak = max(peak, abs(value))
                energy += value * value
                samples += 1
        if peak == 0 or not samples:
            raise ValueError("Renderer produced silence.")
        return {
            "sample_rate": rate,
            "channels": channels,
            "bits_per_sample": bits,
            "duration_s": data_size / align / rate,
            "sample_peak_dbfs": 20 * math.log10(peak),
            "rms_dbfs": 10 * math.log10(energy / samples),
            "sample_peak_at_or_above_full_scale": peak >= (
                1.0 - 2.0 ** (1 - bits) if kind == 1 else 1.0
            ),
        }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("midi", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--sfz", type=Path, default=PIANO / "SalamanderGrandPianoV3.sfz")
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    args = parser.parse_args()
    midi, output = args.midi.resolve(), args.output.resolve()
    manifest_path = output.with_suffix(".render.json")
    log_path = output.with_suffix(".render.log")
    if any(path.exists() for path in (output, manifest_path, log_path)):
        parser.error("Output or provenance exists; choose a fresh output path.")
    midi_metadata = midi_stats(midi)
    instrument, provenance = quiet_instrument(args.sfz.resolve())
    output.parent.mkdir(parents=True, exist_ok=True)
    command = [
        str(args.renderer.resolve()), "--sfz", str(instrument),
        "--midi", str(midi), "--wav", str(output),
        "--samplerate", "44100", "--quality", "10", "--polyphony", "256",
    ]
    env = dict(os.environ)
    env["LD_LIBRARY_PATH"] = str(ROOT / ".runtime/sfizz/root/usr/lib") + (
        ":" + env["LD_LIBRARY_PATH"] if env.get("LD_LIBRARY_PATH") else ""
    )
    started = time.monotonic()
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=600)
    log_path.write_text(result.stdout + result.stderr)
    result.check_returncode()
    stats = wav_stats(output)
    if stats["sample_peak_at_or_above_full_scale"]:
        raise ValueError("Raw sampler output reaches full scale; inspect before listening.")
    manifest = {
        "schema_version": 1,
        "renderer": "sfizz 1.2.3",
        "renderer_source": "https://download.opensuse.org/repositories/home:/sfztools:/sfizz/xUbuntu_22.04/",
        "renderer_sha256": sha256(args.renderer.resolve()),
        "midi_path": str(midi), "midi_sha256": sha256(midi),
        "midi": midi_metadata,
        "output_path": str(output), "output_sha256": sha256(output),
        "instrument": provenance, "command": command,
        "elapsed_s": time.monotonic() - started,
        "audio": stats,
        "postprocessing": "None. Loudness matching belongs to the blind-set preparation step.",
        "pianoteq": "Unavailable: no installed licensed copy; no demo substitution.",
    }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
