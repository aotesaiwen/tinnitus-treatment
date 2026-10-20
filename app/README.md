# Tinnitus Music: Windows app

A desktop app (Tauri 2: Rust backend, React UI) built on the `tmdsp` engine in the parent crate.

## Two modes

**Participant** (default; what a trial participant sees)
- Today's listening minutes against a daily goal, plus a 7-day chart.
- A music player. Each track is processed when it starts playing: resampled to the audio
  device's rate, then run through the assigned condition (notch, sham or passthrough) and
  normalised to −23 LUFS. It **only plays if every verification check passes**. Failing
  tracks are skipped.
- Listening time is logged automatically. Only time the audio is actually playing counts.
- If the computer has not been set up, the participant can run tinnitus matching
  themselves. This is the consumer path: notch arm, ID `self`.

**Clinician** (behind a PIN)
- *Participant:* ID, arm (notch / passthrough / sham), tinnitus pitch (typed in, or taken
  from a matching run), sham frequency, notch width, daily goal. The participant UI never
  shows the arm.
- *Matching:* the full procedure from `tmdsp::matching`, with ear and number of pitch rounds.
- *Music library:* choose a folder of WAV files, then verify every track with this
  participant's settings (checks, notch depth, warnings).
- *Listening log:* daily totals and sessions, with CSV export.

## Data

Everything stays on the computer, in `%APPDATA%\local.tinnitusmusic.research\`:

| File | Contents |
|---|---|
| `profile.json` | Participant ID, arm, pitch, goal |
| `settings.json` | Library folder, PIN hash, volume |
| `listening_log.jsonl` | One line per listening segment: ID, track, start, end, seconds, volume, report file |
| `reports/` | Verification report for every track played (the same JSON as `tmusic process`) |
| `matching/` | Full matching reports, every trial included |

The PIN is a soft lock. Anyone who opens the data folder can read the arm. That's enough
to keep the participant UI blinded, but it doesn't protect against a determined participant.

## Build

Needs Node 20+, the Rust MSVC toolchain (selected automatically by
`src-tauri/rust-toolchain.toml`), Visual Studio Build Tools with the C++ workload, and
WebView2 (built into Windows 11).

```bash
npm install
```

```bash
npm run tauri dev
```

```bash
npm run tauri build
```

`tauri build` produces `src-tauri/target/release/tinnitus-music-app.exe` (portable) and an
NSIS installer under `src-tauri/target/release/bundle/nsis/`.

## Known limits

- WAV only. Files whose name contains `_notch` are ignored, because they're already
  processed.
- No playback calibration. Levels are digital and depend on the Windows volume, so ask
  participants to keep it fixed.
- One participant per computer.
