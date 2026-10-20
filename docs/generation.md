# Music generation

No model download or API account is needed for the offline composer:

```bash
cargo run --release -- generate --style drift --duration 60 --tempo 54 --seed 42 -o drift.wav
cargo run --release -- process -i drift.wav -o drift_processed.wav --center 6000
```

`generate` writes an **unprocessed source WAV**, plus `<output>.generation.json`.
The three styles are `drift`, `focus` and `unwind`. Duration is 30–300 seconds and tempo
45–100 BPM. The composer uses additive synthesis, chord progressions and seeded melodies;
it is explicitly labelled **Offline composer**, not AI. Its seed reproduces a variation
within the same implementation/build; neural-model results may vary across hardware and
server versions.

To use a running ACE-Step 1.5 REST service:

```bash
cargo run --release --features generation-http -- generate \
  --engine ace-step --style focus --duration 60 --tempo 70 --seed 42 \
  --prompt "Soft piano beside a rainy window" -o piano.wav
```

The service defaults to `http://127.0.0.1:8001`. Set `TINNITUS_MUSIC_ACESTEP_URL` to
another service origin, and `ACESTEP_API_KEY` if it requires authentication. Remote
origins require HTTPS. See the [official ACE-Step setup](https://github.com/ace-step/ACE-Step-1.5)
and [REST API contract](https://github.com/ace-step/ACE-Step-1.5/blob/main/docs/en/API.md).
The adapter submits one instrumental WAV request, polls the task, and downloads the
result only from the same service's audio endpoint. It does not send the participant
profile, tinnitus pitch, assigned arm or listening logs.

The desktop app includes the HTTP adapter automatically. Its background jobs support
cancellation, bounded responses, a ten-minute polling deadline, and explicit error states.
Generated tracks are admitted only when all checks pass **and there are no warnings**,
including inadequate source-band energy. Sources remain unprocessed in the library:
playback resamples, applies the current assignment and verifies again. Generation never
starts playback or changes volume.

The library supports search, favorites and restoring a generated track's creation
settings. Music preferences are saved per listener when creating a track or choosing
**Remember settings**. Reusing a recipe restores its seed without starting generation
or playback. **Play new track** starts the completed result on demand. Stable track IDs
keep a selection correct when new music changes the order.
**Save WAV** exports a newly verified listening copy and records its report. Exports
cannot overwrite or enter the source music folders; the default `_listening.wav` suffix
also keeps exported copies out of later library scans. Study exports require clinician access.

For a project-local Linux GPU service, the setup script pins both the upstream code
and model checkpoint revision. It uses an isolated Python 3.12 environment:

```bash
bash scripts/acestep.sh setup
bash scripts/acestep.sh start
```

The API binds only to `127.0.0.1:8001`; stop it with Ctrl-C. `setup` requires `uv`,
network access and space for the Python/CUDA dependencies and model weights.
For a GPU with less memory, enable `ACESTEP_OFFLOAD_TO_CPU=true` and
`ACESTEP_OFFLOAD_DIT_TO_CPU=true` before starting. The script leaves other services alone.

To measure actual generation and source suitability against the application's gate:

```bash
cargo build --release --features generation-http
python3 scripts/validate-generation.py
```

This generates six 30-second tracks across three styles and two seeds, measures
generation time, then checks four notch centers plus passthrough and sham processing.
Raw audio, processed audio, reports and a summary stay under `artifacts/audio/` (ignored
by Git). Exit status is 1 for generation failures and 2 for admission failures.
This is a small technical sample, not a listening study or clinical validation.

## Real-model validation: 20 October 2026

The existing validation record reports a run on an NVIDIA RTX PRO 6000 Blackwell Server Edition, PyTorch 2.10.0+cu128,
using the pinned ACE-Step turbo model, no LM planning, SDPA attention and a 32 GB
allocator cap. See the [recorded measurements](../artifacts/validation/acestep-20261020.json).

| Source style | 30-second sources generated | Listening configurations admitted |
|---|---:|---:|
| Slow drift | 2 / 2 | 10 / 12 |
| Gentle focus | 2 / 2 | 12 / 12 |
| Evening unwind | 2 / 2 | 12 / 12 |

All six sources generated successfully. End-to-end generation and download took
4.134 seconds for the first request, then 2.100–2.126 seconds. The 36 configurations
reuse those six sources: they are not 36 independent musical samples. Both rejections
were Slow drift sources with insufficient flank energy at a 10 kHz center. Numeric
DSP checks passed, but the stricter source-admission gate correctly refused them.
The interface suggests another style and offers a fresh-seed retry.

A 300-second Evening unwind source generated and downloaded in 4.738 seconds;
processing at a 6 kHz center took 3.707 seconds and passed with no warnings
(−23.00 LUFS, −11.15 dBTP). A repeated 30-second request produced the same SHA-256
on this runtime. That does not establish identical output across GPU/runtime versions.
These measurements establish integration and technical audio checks; musical quality,
unintended vocals, listening comfort and clinical benefit are not established by them.
