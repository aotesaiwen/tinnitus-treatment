# tinnitus-music

Tinnitus matching and verified notch processing for tinnitus sound-therapy research and
products.

1. **Matching** (`tmusic match`) measures the listener's tinnitus pitch, loudness and
   minimum masking level, using the standard audiologic sequence.
2. **Notching** (`tmusic process --center <pitch>`) removes the band around that pitch from
   music and proves that it did.

The music comes from an open generative model (ACE-Step 1.5) or from the listener's own
library. This engine handles the part a music model can't guarantee. It removes a
precisely specified band around the tinnitus frequency, sets loudness and peak level, and
**measures the result**. Every output file comes with a JSON report of pass/fail checks
and SHA-256 hashes of both input and output, for audit trails in an RCT.

## Why this split

- **Notched music (TMNMT) is the best-supported music-based approach**, but its effect is
  modest. A 2025 meta-analysis of 14 RCTs (793 patients) found notched music beat ordinary
  music by −8.6 THI points at 3 months ([Jiang et al. 2025](https://pubmed.ncbi.nlm.nih.gov/39992369/)).
  The comparator was an active control, and 5 of the 14 trials had "some concerns" of bias.
- **The October 2026 AAO-HNSF guideline update** still names CBT as the intervention it
  specifically recommends. Sound enrichment is an option, and bimodal neuromodulation is new
  ([AAO-HNSF](https://www.entnet.org/resource/living-with-ringing-in-your-ears-updated-guideline-supports-a-personalized-evidence-based-approach-to-tinnitus/)).
  Position the product as relief and adherence alongside CBT, not as a cure.
- **A prompt can't tell a generator to "leave out 4.2–8.5 kHz at −60 dB".** The notch has
  to be applied, and verified, after generation.
- **Generated music sidesteps licensing.** Notching commercial tracks needs rights;
  ACE-Step 1.5 output can be used commercially (see Licensing).

## Usage

```bash
cargo build --release
```

```bash
./target/release/tmusic process -i song.wav -o song_notched.wav --center 6000
```

```bash
./target/release/tmusic process -i song.wav -o song_control.wav --center 6000 --mode passthrough
```

```bash
./target/release/tmusic design --center 6000 --csv response.csv
```

```bash
./target/release/tmusic analyze -i song.wav --center 6000
```

```bash
./target/release/tmusic bench --minutes 10
```

`process` writes `<output>.report.json` and exits with code 2 if any check fails, so batch
scripts can stop on bad files.

Modes, for trial arms:

| Mode | What it does | Use |
|---|---|---|
| `notch` | Removes the band around `--center` | Treatment arm |
| `passthrough` | Same loudness/peak chain, no notch | Active control (same music, unprocessed) |
| `sham` | Same-shaped notch at `--sham-center` | Sham control. Warns if it overlaps the tinnitus band |

All modes normalise to the same integrated loudness (default −23 LUFS), so the arms can't be
told apart by level.

Defaults: one-octave band geometrically centred on the tinnitus pitch (as in TMNMT),
1/24-octave transitions placed **outside** the band, ≥60 dB stopband, ≤0.1 dB passband
ripple, −1 dBTP ceiling, 32-bit float output. All of these are flags.

## Design

| Stage | Method | Why |
|---|---|---|
| Notch | Linear-phase FIR, Kaiser window. The design loop measures the response and lengthens the filter until the spec is met | The whole octave is stopband, and there's no phase distortion |
| Filtering | FFT overlap-save (`realfft`/`rustfft`, SIMD), channels in parallel; group delay removed | 4,000–6,300× real time; output stays sample-aligned with input |
| Streaming | Same filter for any callback size, fixed latency | Lets a player app apply the notch after decoding |
| Loudness | ITU-R BS.1770-4 gated integrated loudness, any sample rate | Loudness-matched arms |
| True peak | 4× polyphase oversampling | Catches inter-sample peaks |
| Gain | Single linear gain, lowered if the peak ceiling would be exceeded | **No limiter or clipper after the notch**: nonlinear processing would refill the band |
| Verification | Welch PSD, 4-term Blackman-Harris (−92 dB sidelobes), FFT size set so the main lobe fits inside the transition | Measures the notch on the actual signal |

### Checks in every report

| ID | Requirement | Limit |
|---|---|---|
| D1 | Designed attenuation everywhere inside the band | ≥ 60 dB |
| D2 | Designed passband deviation | ≤ 0.1 dB |
| M1 | Measured attenuation on the signal itself | ≥ 57 dB (or residual below −100 dB) |
| M2 | Measured level change outside the notch, 20 Hz–20 kHz | ≤ 0.1 dB |
| M3 | Measured change in the half-octave flanks | ≤ 0.2 dB |
| L1 | Output loudness vs target | ±0.1 LU |
| L2 | Output true peak | ≤ ceiling |
| Q1 | Clipped / non-finite samples | 0 |

Warnings flag inputs where the notch is pointless or the rationale is weak: little energy in
the band, little energy in the flanks (the lateral-inhibition idea needs it), a source
band-limited below the notch (common with generators and lossy codecs), a clipped source, or
a sham overlapping the tinnitus band.

## Verification status

`cargo test --release` runs 32 specification tests (18 notch/loudness, 14 matching), all passing. Notch and loudness:

- Designs meet spec at 44.1/48/96 kHz for centres 250 Hz–14 kHz and widths ½ and 1 octave.
  This is checked independently by exact DTFT at random frequencies.
- Sines: in-band ones drop by ≥ 60 dB. Passband ones come out sample-aligned, within 1.2 %.
- The streaming filter equals the offline filter to 1e-5 at random callback sizes.
- Measured notch on noise is ≥ 60 dB, with ≤ 0.02 dB passband change.
- K-weighting matches the BS.1770-4 48 kHz coefficient table to 1e-8.
- **EBU Tech 3341 loudness cases 1–5 pass within ±0.1 LU** at 48 and 44.1 kHz.
- True peak is within 0.1 dB on an fs/4 sine sampled at 45°.
- Passthrough is a pure gain, and the notch/sham/passthrough arms match within 0.05 LU.

On a music-like 30 s, 16-bit test file, the measured attenuation was 72.6 dB and the
change outside the notch was 0.000 dB.

Speed (24-thread desktop, 10 min of stereo at 48 kHz): filter design ≤ 0.03 s; filtering
one channel 0.1–0.15 s; the full verified pipeline 2.5–3.4 s (≈180–240× real time).

## Tinnitus matching

```bash
./target/release/tmusic match --report match.json
```

```bash
./target/release/tmusic match --sound noise-like --ear left --calibration cal.json
```

```bash
./target/release/tmusic match-sim --n 2000 --pitch-sigma 0.2
```

```bash
./target/release/tmusic stimulus --kind nbn --hz 6000 --level -40 -o nbn.wav
```

Session order (following the VA progressive tinnitus management protocol, Henry et al.):

| Stage | Procedure | Trials (typical) |
|---|---|---|
| Thresholds, 0.5–12 kHz | Pulsed tones (easier to tell from tinnitus); modified Hughson-Westlake (down 10 / up 5 dB; heard twice ascending) | ~60. Skipped if an audiogram is supplied |
| Loudness match at 1 kHz | Louder/softer/equal staircase, 5→2→1 dB steps | ~10 |
| Pitch match ×3 | 2AFC "which is closer?" in log frequency, with comparison tones at equal sensation level. Step halves at each reversal down to 1/6 octave. Random start. Median of the 3 runs | ~25 each |
| Octave-confusion check | Match vs one octave up, then vs one octave down. **Best of three per comparison**, order randomised | 2–6 |
| Threshold + loudness at the pitch | As above | ~18 |
| Broadband-noise threshold, minimum masking level | Ascending 2 dB steps, two runs | ~18 |

Supported options: pure tones or narrowband noise (for hissing tinnitus), either ear (for
one-sided tinnitus, matching is usually done in the other ear), and an optional
calibration file (`{"points": [[hz, dB SPL of a full-scale sine], ...]}`). Without
calibration, levels are dB re full scale and only comparable at a fixed device volume.
Every prompt stays under a hard ceiling: −15 dBFS uncalibrated, 85 dB SPL calibrated. The
renderer refuses anything louder, or anything that would clip.

The report (`match.json`) gives the pitch, loudness (absolute and sensation level), MML,
every trial, `recommended_notch.center_hz` for `tmusic process`, and `retest_recommended`
when the pitch is unreliable (spread > 0.5 octave, split octave vote, or range edge).

The procedures are UI-independent state machines (`Session::next_prompt` /
`Session::answer`), so a mobile or web app can drive them directly.

### Validation (simulated listeners, 2,000 per row)

| Listener model | True pitch inside the 1-octave notch | Misses flagged for retest | Loudness error p90 | MML error p90 | Trials (median) |
|---|---|---|---|---|---|
| Pitch noise 0.2 oct, level noise 1.5 dB | **100.0 %** | 100 % (retest rate 0.7 %) | 1.6 dB | 2.1 dB | 179 |
| Same, audiogram supplied | 99.9 % | — | 1.6 dB | 2.1 dB | **96** |
| Octave confusion, pitch noise 0.1 oct | 94.2 % | 77 % | 1.5 dB | 2.1 dB | 177 |
| Octave confusion, pitch noise 0.2 oct | 78.8 % | 79 % | 1.5 dB | 2.1 dB | 179 |

The octave-confusion rows are limited by the listener, not the procedure. At 0.2 octave of
noise, the simulated listener picks the right octave in a single comparison only ~70 % of
the time, and a best-of-three vote can't do much better than ~78 %. The procedure catches
most of those cases and flags them for retest.

**What this validation does not show.** The listener model is simple: a fixed tinnitus
pitch, Gaussian decision noise, and loudness that follows sensation level (the same
assumption the session uses to balance tones). Real tinnitus pitch matches vary a lot from
test to test because the percept itself drifts. The simulation shows the procedures
converge correctly. Reliability in patients has to be measured in a test-retest study.

## Licensing and regulation (not legal advice)

- **This crate** depends only on MIT/Apache-2.0 crates, so it can ship closed-source.
- **ACE-Step 1.5**: the repository LICENSE is MIT. The Hugging Face model card says
  `license: mit`, states that generated music may be used commercially, and says training
  data was licensed, royalty-free/public-domain and synthetic. That training-data claim is
  the vendor's own. Have counsel review before launch.
- **Avoid** MusicGen weights (CC-BY-NC). Stable Audio is free for commercial use only
  below $1M annual revenue.
- **Claims decide classification.** In the US, "relief of tinnitus" sound generators have
  been cleared as Class II devices under 21 CFR 874.3400 (e.g. the
  [Sound Options 510(k)](https://www.accessdata.fda.gov/cdrh_docs/pdf16/K161562.pdf)). A
  relaxation/sound-enrichment app with no tinnitus-treatment claims is the wellness route.
  In the EU, software intended to treat falls under MDR Rule 11 (class IIa or higher).
- **Patents**: check freedom to operate for notched-music and CR-neuromodulation patents
  before commercial launch.
- **Listening level**: LUFS is a digital level. It doesn't set dB SPL. The app has to cap
  playback volume (WHO/ITU-T H.870 safe-listening guidance) and set the level relative to
  the listener's tinnitus loudness.

## Known limits

- WAV only (PCM 8/16/24/32, float 32/64). Decode MP3/FLAC upstream.
- **Untested:** whether lossy re-encoding (MP3/AAC) of notched audio adds quantisation noise
  inside the band. Until that's measured, ship lossless files or apply the notch at playback
  with `StreamingFilter`.
- The streaming filter buffers about 4× the filter length (around 0.1 s at 4 kHz, around
  1.5 s at 250 Hz). That's fine for music playback but not for live monitoring.
- No clinical claims follow from the engine's precision. The 2025 evidence shows small,
  bias-limited effects.

- The interactive `match` command plays sounds with the OS player (PowerShell SoundPlayer
  on Windows, `afplay`, `aplay`). It's a research and demo front end, not a product UI.
  Residual inhibition testing is not implemented.

## Next

1. Generate an ACE-Step 1.5 library on the local RTX 4090, then run every track through
   `process` and reject files that fail checks or warnings (e.g. band-limited output).
2. A test-retest pilot of `match` with the ENT collaborator, comparing it against clinic
   audiometer matches.
3. Mobile/web app: compile `Session` and `StreamingFilter` to iOS/Android/WASM.
