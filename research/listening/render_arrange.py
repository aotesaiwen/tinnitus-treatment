#!/usr/bin/env python3
"""Documented piano arrangement: omit MIDI percussion, retain all pitched notes.

This is not a new model generation. Original multitrack output stays untouched;
every non-percussion event keeps its absolute tick, channel and data except that
program-change values become acoustic piano (0). Tempo/meta events stay intact.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct

from render_piano import midi_stats, sha256


def variable(value):
    if not 0 <= value <= 0x0FFFFFFF:
        raise ValueError("MIDI delta is outside the standard range.")
    encoded = [value & 0x7F]
    while value := value >> 7:
        encoded.insert(0, (value & 0x7F) | 0x80)
    return bytes(encoded)


def parse(path):
    data = path.read_bytes()
    if data[:4] != b"MThd":
        raise ValueError("Expected MIDI header.")
    header_size = int.from_bytes(data[4:8], "big")
    header = data[:8 + header_size]
    track_count = int.from_bytes(data[10:12], "big")
    offset = 8 + header_size
    tracks = []
    for _ in range(track_count):
        if data[offset:offset + 4] != b"MTrk":
            raise ValueError("Expected MIDI track.")
        size = int.from_bytes(data[offset + 4:offset + 8], "big")
        track = data[offset + 8:offset + 8 + size]
        if len(track) != size:
            raise ValueError("Truncated MIDI track.")
        offset += size + 8
        cursor, running, tick = 0, None, 0
        events = []

        def read_variable():
            nonlocal cursor
            value = 0
            for _ in range(4):
                byte = track[cursor]
                cursor += 1
                value = (value << 7) | (byte & 0x7F)
                if not byte & 0x80:
                    return value
            raise ValueError("Invalid variable-length value.")

        while cursor < len(track):
            tick += read_variable()
            status = track[cursor]
            if status & 0x80:
                cursor += 1
            elif running is not None:
                status = running
            else:
                raise ValueError("Missing running status.")
            if status in (0xF0, 0xF7, 0xFF):
                running = None
                event_start = cursor
                if status == 0xFF:
                    cursor += 1
                size = read_variable()
                cursor += size
                if cursor > len(track):
                    raise ValueError("Truncated system/meta event.")
                message = bytes([status]) + track[event_start:cursor]
            elif 0x80 <= status <= 0xEF:
                running = status
                size = 1 if status & 0xF0 in (0xC0, 0xD0) else 2
                message = bytes([status]) + track[cursor:cursor + size]
                if len(message) != size + 1 or any(value > 127 for value in message[1:]):
                    raise ValueError("Invalid channel event.")
                cursor += size
            else:
                raise ValueError("Unsupported system event.")
            events.append((tick, message))
        tracks.append(events)
    return header, tracks


def arrange(input_path, output_path):
    record_path = output_path.with_suffix(".arrangement.json")
    if output_path.exists() or record_path.exists():
        raise ValueError("Choose fresh arrangement/provenance paths.")
    original_hash = sha256(input_path)
    original_stats = midi_stats(input_path, require_piano_range=False)
    header, original = parse(input_path)
    arranged, removed_notes, removed_events, changed_programs = [], 0, 0, 0
    for track in original:
        kept = []
        for tick, event in track:
            status = event[0]
            if 0x80 <= status <= 0xEF and status & 0x0F == 9:
                removed_events += 1
                removed_notes += status & 0xF0 == 0x90 and event[2] != 0
                continue
            if status & 0xF0 == 0xC0:
                changed_programs += event[1] != 0
                event = bytes([status, 0])
            kept.append((tick, event))
        arranged.append(kept)
    output = bytearray(header)
    for track in arranged:
        contents = bytearray()
        previous = 0
        for tick, event in track:
            contents.extend(variable(tick - previous))
            contents.extend(event)
            previous = tick
        output.extend(b"MTrk" + struct.pack(">I", len(contents)) + contents)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with output_path.open("xb") as stream:
        stream.write(output)
    # Semantic audit is stricter than comparing note counts: absolute ticks,
    # note-on/off velocities, sustain controls, and every meta event must match.
    _, reparsed = parse(output_path)
    if reparsed != arranged:
        raise AssertionError("Arrangement did not preserve the audited event sequence.")
    arranged_stats = midi_stats(output_path)
    if arranged_stats["note_on_count"] != original_stats["pitched_note_count"]:
        raise AssertionError("A pitched note was lost or added.")
    if sha256(input_path) != original_hash:
        raise AssertionError("Source MIDI changed during arrangement.")
    manifest = {
        "schema_version": 1,
        "kind": "piano arrangement of a multitrack model output",
        "is_successful_solo_piano_model_generation": False,
        "input": str(input_path), "input_sha256": original_hash,
        "output": str(output_path), "output_sha256": hashlib.sha256(output).hexdigest(),
        "source_midi": original_stats,
        "arranged_midi": arranged_stats,
        "transformations": {
            "percussion_channel_zero_based": 9,
            "percussion_note_ons_omitted": removed_notes,
            "percussion_channel_events_omitted": removed_events,
            "nonzero_pitched_program_changes_mapped_to_zero": changed_programs,
            "pitched_note_pitch_onset_duration_velocity_preserved": True,
            "tempo_and_other_meta_events_preserved": True,
            "remaining_controllers_preserved": True,
            "musical_smoothing_or_note_replacement": False,
        },
        "rendering": "All remaining pitched tracks use the same Salamander Grand Piano V3 instrument.",
    }
    record_path.write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    arrange(args.input.resolve(), args.output.resolve())


if __name__ == "__main__":
    main()
