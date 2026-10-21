# Tinnitus Music

A reproducible music generation and tinnitus listening research toolkit. The Rust
engine processes and validates audio; the Tauri / React desktop app handles
composition, playback, participant settings, and listening logs.

**Workflow: generate or import music → validate audio → process with current
settings → start playback → log listening.** Passing audio checks does not
demonstrate clinical efficacy. This project has not completed patient studies or
clinical product validation.

## Quick start

Requires stable Rust. The offline composer needs no model or API:

```bash
cargo build --release --locked
./target/release/tmusic generate --style drift --duration 60 --tempo 54 --seed 42 -o drift.wav
./target/release/tmusic process -i drift.wav -o drift_listening.wav --center 6000
```

`generate` saves the original WAV and generation parameters. `process` saves the
processed WAV, validation results, and SHA-256 hashes. Exit codes: `0` success,
`1` execution error, `2` audio validation failed. Run `tmusic --help` for analysis,
filter design, tinnitus matching, and simulation commands.

## Project structure

| Path | Purpose |
|---|---|
| `src/` | DSP, WAV I/O, resampling, matching, offline composition, and optional ACE-Step adapter |
| `app/src/` | React studio, library, player, and clinical settings interface |
| `app/src-tauri/src/` | Desktop state, playback, matching, library, clinical commands, and persistence |
| `tests/`, `app/tests/` | Signal validation, HTTP protocol, persistence, and UI regression tests |
| `scripts/` | Optional model service setup and generation validation |
| `artifacts/validation/` | Small, traceable records of historical technical measurements |

- [Desktop development and data](app/README.md)
- [Generators, ACE-Step setup, and historical measurements](docs/generation.md)
- [Audio processing, validation metrics, and matching](docs/audio-engine.md)
- [Blind music listening demo](listening-demo/README.md) and [local model comparison](research/listening/README.md)
- [Music research lab: candidate selection, preference learning, and paired controls](https://github.com/aotesaiwen/tinnitus-treatment/blob/codex/music-research-lab/research/README.md)

Generated sources retain their original audio. Playback and export reprocess it
with the current participant settings. Clinical mode controls generation and
export for research participants and hides group assignments from the interface.
The PIN locks the interface; it does not encrypt data.

## Verification

```bash
cargo test --locked --features generation-http
cargo clippy --locked --all-targets --features generation-http -- -D warnings
cd app
npm ci
npm run build
npx playwright install chromium
npm test
```

HTTP tests use local mock servers; browser tests mock Tauri IPC. Audio output on
physical desktop devices, sound pressure level calibration, and extended use
require separate validation. Model services are optional; routine development
does not require downloading weights. CI checks the Rust core, browser UI, and
Windows desktop compilation.

## Limitations

- Input must be WAV; decode other formats first. Lossless output preserves the notch.
- Generation supports 30–300 seconds. The offline composer is not a neural model.
- LUFS / dBTP measure digital audio, not the actual sound pressure level at the headphones.
- Historical simulations and GPU measurements are technical validation, not evidence of patient benefit or reproducibility across hardware.
- Before release, review model and data licenses, clinical evidence, and product requirements for the target market.
