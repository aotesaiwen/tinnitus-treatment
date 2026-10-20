# Paired control materials

Create `notch`, `passthrough`, and `sham` conditions from the same source music,
using fixed defaults for loudness, peak level, and notch width. Input audio is
read-only. The listening directory is published only when all three conditions
pass DSP checks without warnings, source and output hashes match their records,
and the output loudness range is no greater than 0.2 LU.

```bash
python3 -m research.paired --binary target/release/tmusic \
  --source source.wav --center 6000 --sham-center 1500 \
  --seed 42 --output artifacts/audio/paired-demo
```

- `listening/`: neutral filenames and `schedule.json`. All six permutations of
  the three clips are included. Each condition appears twice in each position,
  and each directed adjacent pair appears twice.
- `private/`: condition mapping, processing reports, parameters, and source
  hashes. Keep these for researchers; do not distribute them to evaluators with
  the listening directory. The directory name provides no access control or
  encryption.
- `summary.json`: overall pass status, loudness difference, and mapping file location.
- Existing output directories are rejected. Failed validation exits with code
  `2`, retaining audit materials without creating `listening/`. Execution errors
  exit with code `1`.

The DSP report's `output_path` refers to the staging path used during processing.
After validation, audio moves to `listening/`; `schedule.json` records its final
location and hash.

This controls the confound of using different music in different conditions,
but is not a complete clinical trial design. Six orders are not six independent
samples, and listeners may still hear processing differences. A formal study
must also specify independent music and participant sample sizes, allocation
and concealment methods, handling of carryover effects, predefined outcomes,
and an analysis plan. This tool does not allocate real participants, start
playback, or calculate treatment efficacy.
