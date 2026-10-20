# Audio engine reference

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

All modes target the same integrated loudness (default −23 LUFS). Peak constraints
can reduce gain; inspect the report for loudness differences.

Defaults: one-octave band geometrically centred on the tinnitus pitch (as in TMNMT),
1/24-octave transitions placed **outside** the band, ≥60 dB stopband, ≤0.1 dB passband
ripple, −1 dBTP ceiling, 32-bit float output. All of these are flags.

## Design

| Stage | Method | Why |
|---|---|---|
| Notch | Linear-phase FIR, Kaiser window. The design loop measures the response and lengthens the filter until the spec is met | The whole octave is stopband, and there's no phase distortion |
| Filtering | FFT overlap-save (`realfft`/`rustfft`, SIMD), channels in parallel; group delay removed | Output stays sample-aligned with input |
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

`cargo test --release` covers the matching, DSP and offline composer.
`cargo test --features generation-http` also runs the ACE-Step HTTP contract tests against
local fixture servers (no real model). The generation suite covers reproducibility,
fades, finite samples, cancellation, invalid inputs, all three processing modes, API
errors, credential handling, download origin checks and malformed audio.
Notch and loudness coverage includes:

- Designs meet spec at 44.1/48/96 kHz for centres 250 Hz–14 kHz and widths ½ and 1 octave.
  This is checked independently by exact DTFT at random frequencies.
- Sines: in-band ones drop by ≥ 60 dB. Passband ones come out sample-aligned, within 1.2 %.
- The streaming filter equals the offline filter to 1e-5 at random callback sizes.
- Measured notch on noise is ≥ 60 dB, with ≤ 0.02 dB passband change.
- K-weighting matches the BS.1770-4 48 kHz coefficient table to 1e-8.
- **EBU Tech 3341 loudness cases 1–5 pass within ±0.1 LU** at 48 and 44.1 kHz.
- True peak is within 0.1 dB on an fs/4 sine sampled at 45°.
- Passthrough is a pure gain, and the notch/sham/passthrough arms match within 0.05 LU.

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
