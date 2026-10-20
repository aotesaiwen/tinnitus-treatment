# Desktop app

Tauri 2 + React, backed by the root `tmdsp` crate. The packaged target is Windows (NSIS).

## Develop

Use Node 22.12+ and Rust stable. Windows builds require the MSVC toolchain,
Visual Studio C++ Build Tools and WebView2. Linux development additionally requires
GTK 3, WebKitGTK 4.1 and ALSA development packages; the Windows installer is built on Windows.

From `app/`:

```bash
npm ci
npm run tauri dev
```

```bash
npm run build
npm test
npm run tauri build
```

Install Chromium once with `npx playwright install chromium`. Tests mock Tauri IPC;
they do not validate a physical audio device. `cargo check --locked --manifest-path
app/src-tauri/Cargo.toml` from the root checks the native backend with the host toolchain.
The installer is written under `src-tauri/target/release/bundle/nsis/`.

## Workflows

- **Participant:** choose a mood, compose offline or use ACE-Step, wait for admission
  checks, then explicitly play. Search, favorites, recipe reuse and verified WAV export
  are available. A study participant's clinician controls generation and export.
- **Clinician:** unlock with a PIN, configure participant and listening condition, run
  matching, choose and verify a source library, and export the current participant's logs.
- **Matching:** without a configured participant, self setup creates the `self` profile.
  Existing study assignments require clinician access to change.

Generation never starts playback or changes volume. Sources are admitted only when all
checks pass without warnings; playback resamples and rechecks the assigned processing.
Only audible playback time counts. Switching participants isolates preferences and logs.

See [generation setup and limitations](../docs/generation.md) for ACE-Step configuration.
Cancellation discards local results but does not cancel a submitted remote model job.
In-flight HTTP requests may take 60 seconds to return; jobs do not resume after restart.

## Code boundaries

| Module | Responsibility |
|---|---|
| `lib.rs` | Shared state, processing helpers and application lifecycle |
| `status.rs` | Participant-visible status; one library representation with stable IDs |
| `player.rs`, `audio.rs`, `selection.rs` | Playback commands, audio device and selection rules |
| `matching.rs` | Matching commands and result presentation |
| `library.rs`, `export.rs` | Preferences, favorites, library checks and verified export |
| `clinician.rs` | Access, profiles and participant-scoped log export |
| `generation.rs`, `store.rs` | Background generation and persistence |

## Data

Default Windows location: `%APPDATA%\local.tinnitusmusic.research\`.
`TINNITUS_MUSIC_DATA_DIR` selects a separate data folder.

| File / folder | Contents |
|---|---|
| `profile.json`, `settings.json` | Current assignment, library folder, PIN hash and volume |
| `library_preferences.json` | Preferences and favorite IDs per participant |
| `listening_log.jsonl` | Listening segments tagged with participant ID |
| `reports/`, `matching/` | Playback/export verification and matching reports |
| `generated/library/` | Unprocessed admitted WAV sources |
| `generated/records/` | Recipe, provider metadata, hashes, assignment snapshot and verification |
| `generated/rejected/` | Rejected generation records |

The PIN is a soft UI lock; disk access can reveal study assignments. ACE-Step receives
music preferences, not profiles, tinnitus pitch, assignments or listening logs.
Sources containing `_notch` or ending in `_listening` are excluded from library scanning.
Export into source folders is blocked. Playback has no physical sound-level calibration.
