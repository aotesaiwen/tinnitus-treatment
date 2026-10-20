# From listening feedback to recipe recommendations

The desktop player now offers a music rating while a generated track is playing
or paused. Listeners can rate it from 1 to 5 and optionally answer whether the
music was uncomfortable. Saving is explicit. Responses are local and do not
change playback, volume, processing settings, or a study assignment.

The backend checks the current participant and track when saving. Imported audio,
stopped/loading playback, and matching sessions are not eligible. Study listeners
can submit ratings without seeing recipes or processing conditions.

## Recorded events

`music_feedback.jsonl` lives beside `profile.json` in the app data directory. Each
append-only event contains `schema_version`, `event_id`, `participant_id`,
`track_id`, `recipe` (provider and full generation request), `rating`,
`discomfort`, and `created_at`. Discomfort is `true`, `false`, or `null` when
unanswered. No treatment arm or tinnitus pitch is copied into the event.
Saving publishes the complete ledger atomically while holding the app lock;
a failed publication preserves the previous file. Incomplete final records
are refused rather than extended by another save.

The current UI can collect another response after reopening a track. Separate
event IDs are separate responses; they are not assumed to be independent people
or independent listening sessions. There is no automatic symptom monitoring or
adverse-event notification in this prototype.

## Import for one listener

Run from the repository root, substituting the actual app-data path:

```bash
python3 -m research.feedback \
  --ledger /path/to/app-data/music_feedback.jsonl \
  --participant self --output artifacts/audio/feedback-import
python3 -m research.preferences \
  --input artifacts/audio/feedback-import/preferences.json \
  --participant self --seed 42
```

The output directory must be new. `preferences.json` contains only that listener's
catalog and usable responses. `provenance.json` maps included events to tracks,
generation seeds and timestamps. Keep both files private and outside version
control. Recipe identifiers hash the provider, style, tempo, duration and prompt;
variations with different audio seeds share a recipe, while original seeds remain
in provenance.

The importer validates the event schema, refuses conflicting duplicate event IDs,
and counts identical duplicate events once. It reports how many unanswered
discomfort responses it skipped. It never converts an unanswered question to
`false`. A malformed ledger or a changing source file aborts import. If no usable
responses remain, it reports an error rather than inventing preference evidence.
An explicit discomfort report remains an exclusion in the preference prototype,
even if other responses to that recipe were positive.

This completes data collection and an offline recommendation path. The desktop
app does not automatically consume the recommendation or generate a new track.
The [preference model's limitations](preferences.md) still apply, including
selection bias and correlated repeat ratings. Human responses are needed before
making any claim about preference prediction or listening comfort.
