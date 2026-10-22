# Music generation comparison

This experiment compares actual outputs for the brief “gentle solo piano.” It
supports listening decisions about these excerpts, not a general ranking of
models. The web interface is in [`listening-demo`](../../listening-demo/README.md).

## Routes

| Route | What is actually tested |
| --- | --- |
| Aria | Two fixed-seed piano continuations from one original eight-bar MIDI prompt. The listening excerpt excludes the prompt. |
| Text2midi | Two fixed-seed text-to-MIDI generations, adapted to piano as described below. |
| Salamander Grand Piano V3 / sfizz | One common sampled piano for both MIDI routes and the authored control. |
| ACE-Step 1.5 turbo | One existing direct-audio output, seed 42022, eight inference steps, language model disabled. No new ACE generation. |
| Authored control | The original eight-bar MIDI prompt, rendered with the same piano. Authored in code, not output from a trained music model. |
| Stable Audio 3 Small Music | Two fixed-seed direct-audio generations from verified official weights. |
| Stable Audio 3 Medium | Two fixed-seed direct-audio generations, using the same prompt and seeds as Small Music. |
| Lyria RealTime | Two actual 40-second streams from Google's free experimental API, captured unchanged. This is not Lyria 3.5. |
| Eleven Music v2.5 | Not generated: API credentials are valid, but the current free subscription does not include the paid-only Music API. |
| Lyria 3.5 | Not generated: API credentials are valid, but this model has no free API tier. Billing was not enabled. |
| Pianoteq | Not rendered: no installed licensed copy. A restricted trial is not used as a valid comparison. |

No unavailable route is represented by another model's audio. The MIDI models
use different conditioning interfaces: Aria receives notes, Text2midi receives
text. Instrument consistency does not make their tasks identical.

## Text2midi's instruction failure

Both original outputs contain multiple instruments and percussion despite the
solo-piano requests. The first contains 464 note-ons (316 pitched, 148 drums);
the second contains 472 (396 pitched, 76 drums). The first also misses the tempo
request. Official checkpoint hashes, tokenizer bytes, and shared model-class
implementations were checked; no implementation mismatch was found.

The listening versions are **piano arrangements of those outputs**: percussion
events are omitted and all remaining tracks use the common piano. Pitched notes,
onsets, durations, velocities, and tempo events are retained. Original MIDI and
tokens remain unchanged. These samples do not demonstrate successful adherence
to a solo-piano prompt. The checkpoint's token vocabulary has no sustain-pedal
events; none are invented afterward.

## Audio preparation

1. Generate both predetermined seeds for each new model. Retain their full
   original outputs and provenance; do not choose a preferred retry.
2. Render MIDI through sfizz 1.2.3 with the original Salamander Grand Piano V3,
   44.1 kHz / 16-bit sample library. Omit mechanical key/pedal-noise regions
   uniformly; retain piano-note, string-release, and resonance samples.
3. Take the first 30 seconds of each source. For Aria, the source is the
   generated-only continuation. Add the same 20 ms edge fades to every excerpt.
4. Resample to 44.1 kHz stereo when necessary. Use `tmusic process --mode
   passthrough` to match integrated loudness to -23 LUFS with gain alone and a
   -1 dBTP ceiling. No frequency shaping, notch, denoising, or compression.
5. Export 16-bit lossless FLAC using opaque filenames. Verify duration, decoding,
   loudness, peak level, and file hashes. The browser shuffles them again.

The equal-length excerpt may interrupt a phrase. Loudness is digital; the page's
volume control and the listener's equipment still determine playback level.
Technical checks do not assert that a piece sounds good. That is the purpose of
the listening round.

## Reproduce

### Optional provider access

Browser sign-in does not authorize local generation scripts. Run the Chinese
hidden-input setup with the system Python (no extra packages required):

```sh
python3 scripts/configure-music-access.py
python3 scripts/configure-music-access.py --status
```

It accepts a Hugging Face read token, a Gemini API key, and an ElevenLabs API
key. Each is optional; Enter keeps the existing value or skips the provider.
Keys are saved only in the Git-ignored `.runtime/music-access/credentials.json`
with directory permissions `0700` and file permissions `0600`. The setup does
not call paid APIs, download weights, or verify account entitlements. Runners
must explicitly load this file or receive the corresponding environment
variables; the setup does not change global shell configuration.

Create keys at [Hugging Face](https://huggingface.co/settings/tokens),
[Google AI Studio](https://aistudio.google.com/apikey), and
[ElevenLabs](https://elevenlabs.io/app/api/api-keys).
Stable Audio also requires accepting the model access terms for
[Medium](https://huggingface.co/stabilityai/stable-audio-3-medium) and
[Small-Music](https://huggingface.co/stabilityai/stable-audio-3-small-music).
The [Eleven Music API requires a paid account](https://elevenlabs.io/docs/eleven-api/guides/cookbooks/music).
Pianoteq is optional and requires a purchased license; the existing Salamander
renderer does not require a serial number.

### Local generation

Generation runners use isolated runtimes under `.runtime/`; model weights and
large audio files are excluded from Git. Setup details live in `aria-setup.md`,
`text2midi-setup.md`, `stable-audio-setup.md`, and `lyria-realtime-setup.md`.

```sh
cargo build --release --locked
python3 research/listening/render_setup.py
python3 research/listening/render_piano.py INPUT.mid OUTPUT.wav
.runtime/ACE-Step-1.5/.venv/bin/python research/listening/build_blind_set.py research/listening/sources.json
```

For the two Text2midi outputs, first run `render_arrange.py RAW.mid ARRANGED.mid`,
then render the explicitly arranged MIDI. That script verifies retained events
at absolute MIDI ticks and records every omitted percussion event. The separate
renderer timing audit confirmed that sfizz follows the tempo map; its longer
WAV tails are the piano samples' natural decay.

The latest dataset has twelve excerpts. The original six-excerpt dataset and its
browser-local scores remain accessible at `round-1/`; its sample IDs and storage
version are unchanged. Each dataset uses a separate storage key, so revealing or
resetting one round does not change the other. Already familiar excerpts cannot
be made unfamiliar merely by shuffling them again.

The final command needs NumPy, SciPy, and SoundFile; the existing ACE environment
supplies these libraries only and does not generate music. A source list records
each input path, offset, source label, and configuration. Missing or short audio,
duplicate source audio, unexpected processing, or inability to match loudness
causes failure instead of replacement or silent padding.

Full run records are under `artifacts/audio/blind-20261021/`: raw model outputs,
token IDs, code/checkpoint revisions, renderer hashes, arrangement changes, and
`prepared/measurements.json`. A compact reproducibility record is committed in
[`run-record.json`](run-record.json) for round one and
[`run-record-v2.json`](run-record-v2.json) for round two. Original and derived MIDI files are also
retained in [`midi/`](midi/); full raw WAV files remain outside Git. The page
fetches `reveal.json` only after explicit disclosure.

## Attribution

Piano samples: **Salamander Grand Piano V3 by Alexander Holm**, licensed
[CC BY 3.0](https://creativecommons.org/licenses/by/3.0/), from the
[original archive](https://archive.org/details/SalamanderGrandPianoV3).
The rendering preset omits mechanical key/pedal-noise layers. Audio excerpts are
cropped, edge-faded, and gain-matched as described above.

Official model sources: [Aria](https://github.com/EleutherAI/aria),
[Text2midi](https://github.com/AMAAI-Lab/Text2midi), and
[ACE-Step 1.5](https://github.com/ace-step/ACE-Step-1.5).

## Repertoire processing (2026-10-22)

The published full Chopin rendition now has client-side sound controls. Its
source MIDI, FLAC, attribution, renderer and original gain-only render record
remain unchanged. `listening-demo/dist/repertoire/audio-engine.js` implements
broad ±6 dB shelves, one music path for original/processed comparison, pink-noise
background and a transparent-below-threshold peak guard. There is no new music
generation, no notch, no model/provider selection and no change to note releases.
The selected Heolstor arrangement remains `awaiting_score_file`.

`calibrate_repertoire.py` measures full-track BS.1770 loudness for the Web Audio
shelf grid using NumPy/SciPy/SoundFile, following the same K weighting and gating
as `src/loudness.rs`. Regenerate its small static calibration module with:

```sh
.runtime/ACE-Step-1.5/.venv/bin/python research/listening/calibrate_repertoire.py
```

This table is specific to the existing Chopin audio hash and must be recalibrated
if that audio or the shelf frequencies/ranges change. Background is not part of
loudness matching; interpolation and local musical passages can differ. See
`listening-demo/README.md` for browser lifecycle, persistence and verification.
`repertoire-player-validation.json` records this iteration's measured signal
results and coverage. Listening comfort or clinical benefit is not inferred
from technical tests. Prior listener scores and preferences are unchanged.
