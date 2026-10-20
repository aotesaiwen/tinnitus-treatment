# Technical best-of-N music selection

This prototype asks whether generating several seeds increases the chance of obtaining
music that passes the existing DSP admission gate. It selects using measured digital
audio properties. It does not measure preference, listening comfort or tinnitus relief.
No patient outcome enters the ranking.

Run from the repository root with Python 3.10 or newer; only the standard library is used:

```bash
cargo build --locked
python3 -m research.selection \
  --binary target/debug/tmusic \
  --output artifacts/audio/selection-example \
  --engine composer --style drift --duration 30 --tempo 60 \
  --seed 42 --candidates 4 --center 6000
```

The output directory must not exist, including as a symlink. A run never reuses an old
report or overwrites another experiment. `--candidates` accepts 1–16, duration 30–300
seconds, tempo 45–100 BPM, and center 20–16000 Hz. Seeds are consecutive unsigned
32-bit integers starting at `--seed`; overflow is refused. The default values are shown
above. Styles are `drift`, `focus`, and `unwind`.

`--engine ace-step` uses the existing optional HTTP provider. Build with
`cargo build --locked --features generation-http` and configure its service as described
in [generation setup](../docs/generation.md). Each candidate is a separate request, and
each command has a 900-second deadline. Provider errors and timeouts reject that attempt;
the remaining seeds still run. A run makes at most 16 generation requests and does not
automatically retry, start playback, or tune the listener's frequency.

Every source passes through `tmusic process` with explicitly recorded parameters:
one-octave notch at the requested center, 1/24-octave transition, 60 dB attenuation,
0.1 dB ripple, −23 LUFS target, −1 dBTP ceiling, and float WAV output. Unsupported
frequency/sample-rate combinations remain subject to the engine's checks.

A candidate is admitted only when both commands exit zero, its manifest matches the
requested recipe and source hash, its report matches the preserved waveforms, `all_pass`
is true, every individual check passes, and the warning list is empty. Missing or malformed
artifacts, non-finite metrics, inconsistent targets, and modified source files also reject
the candidate. The output loudness and peak tolerances are independently checked against
the pipeline's 0.1 LU and 0.05 dB tolerances.

Admitted candidates are ordered by:

1. Smallest absolute deviation from the −23 LUFS target.
2. Greatest headroom below the −1 dBTP ceiling.
3. Smallest seed, to resolve exact ties reproducibly.

This deliberately transparent ranking is a technical baseline. Tiny LUFS differences can
determine the order and should not be interpreted as meaningful improvements in music.
Larger headroom is not a validated surrogate for preference or clinical efficacy.
Rejected candidates never become a fallback selection.

`summary.json` contains the recipe, executable SHA-256, platform and Python version,
full command arguments, per-stage timings and exit codes, generation manifests, source
and output hashes, checks, warnings, failures, ranking, and selected candidate ID.
Paths inside attempts are relative to the output directory. Each candidate directory
preserves `source.wav`, its generation manifest, `processed.wav`, `report.json`, and
command stdout/stderr when those files were produced. The selected ID points to these
files; there is no extra lossy conversion or replacement of the source.

The summary is checkpointed after each attempt. An interrupted run can leave
`status: "running"` and a partial final candidate directory; only a completed summary
with `status: "selected"` is a finished selection. Reproduce a run in a new directory.
The same seed and binary support composer repeatability; model versions, hardware and
provider configuration can still affect ACE-Step. Its returned model metadata is
preserved, but service credentials and environment variables are not copied.

Exit status is `0` when a candidate was selected, `2` when all attempts were rejected,
and `1` for setup or filesystem errors. Standard argument-parser syntax errors also
return `2`, before a summary is created. Inspect `status` and the individual attempts
instead of interpreting exit status alone.

```bash
python3 -m unittest research.tests.test_selection -v
```

The tests cover warnings, nonzero command status, inconsistent checks, source hashes,
invalid metrics, provider failure isolation, deterministic ranking, empty admission,
invalid parameters, and refusal to overwrite existing experiments.

A useful next experiment is a preregistered comparison of admission rates at N = 1, 2,
4 and 8 across fixed seeds, styles and frequencies, with listening preference evaluated
separately. Selecting a favorable seed must not hide the rejected attempts or be presented
as independent evidence of treatment benefit.
