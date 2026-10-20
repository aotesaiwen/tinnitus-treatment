# Generation matrix

Run a fixed comparison of the additive composer and ACE-Step prompt variants.
The supplied plan has nine recipes, two disjoint four-seed splits and four
processing centers: **72 source attempts and 288 planned checks**.

```sh
cargo build --release --features generation-http
python3 -m research.benchmark \
  --config research/examples/generation-matrix.json \
  --binary target/release/tmusic \
  --output /tmp/tmusic-generation-matrix
```

The ACE-Step service must already be available to the executable. The CLI uses
`TINNITUS_MUSIC_ACESTEP_URL` and, where needed, `ACESTEP_API_KEY`. The benchmark
does not copy environment variables or credentials into its plan.

The baseline and overtone variants share each style's description, tempo,
duration and seed. The latter adds acoustic details, brushed cymbals and airy
strings. These are compositional hypotheses about technical band coverage,
not instructions to a model to deliver a treatment. The executable also adds
its fixed style caption and instrumental/dynamics instructions; preserve its
binary hash and generation metadata when interpreting comparisons.

## Evidence and retention

`plan.json` is written before the first generation, including the full config,
fixed processing parameters, execution order, binary/code SHA-256 hashes,
Python/platform information and command timeout. `summary.json` is atomically
updated after each source. Each source also receives its own summary. Outputs
include command arguments, exit statuses, timings, stdout/stderr, source WAVs,
generation manifests, reports and processed waveform hashes.

Each raw source is generated once and the same file is processed at all
centers. Generation is accepted only with a successful exit and matching
manifest hash, request and engine. Admission requires both successful command
exits, matching input/output waveform hashes, individually passing checks,
`all_pass: true`, no warnings and valid loudness/peak measurements. A failed
generation counts as a failed attempt at every planned center. Missing or
malformed reports never pass. Per-case errors do not stop later cases.

By default only newly generated processed WAVs whose source and output hashes
match the saved report are removed. This includes a technically rejected WAV
with an otherwise verified report; its raw source and report remain. Outputs
with missing/malformed reports or mismatched hashes are retained for diagnosis.
Pass `--keep-processed` to retain every processed WAV for inspection. Existing
output paths, including dangling symlinks, are refused. Each stage times out
after 900 seconds. Exit 1 indicates setup failure; exit 2 means the matrix
completed with no admitted cases; exit 0 means at least one case was admitted.
Interruption leaves completed-source summaries available; an interrupted
source may have additional raw artifacts and is not counted as completed.

## Reading the result

`summary.json` contains source records, `rows` by **recipe/split/center**, and
`budget_rows` at those same levels. `admission.csv` and `budgets.csv` provide
compact tables; JSON retains warning counts, omitted seeds and individual
budget blocks. `attempted` includes generation failures; `generated` requires
manifest verification. `planned` stays fixed while an experiment runs.

For each budget N, seeds are divided in their declared order into disjoint
blocks of N. The first admitted seed is selected within a block; a failed
generation consumes an attempt. Partial trailing blocks are omitted and
their seeds explicitly recorded. Full blocks awaiting sources are reported
as incomplete. Different budgets reuse seeds, so their results are correlated.
`estimated_serial_s` sums measured generation and single-center processing
costs up to first admission (or exhaustion); it estimates a sequential search,
not the actual wall time of the full matrix. The root `generation_s` counts
each source once, and `processing_s` counts all executed processing stages.
Generation time repeats across center-specific rows for context and must not
be summed across centers.

Compare baseline versus overtone prompts **within each style, split and
center**, including admission counts, failure reasons and cost. Centers are
repeated measurements of the same sources, not independent samples. Recipe
execution order is fixed, so service load or warmup may confound timing.
Four seeds per split give only exploratory evidence. Keep development and
holdout results separate; changing the plan after reading holdout results
requires a new plan and new seeds for an untouched holdout.

The seed split is a local predeclared plan, **not external preregistration**.
Model seeds and hashes document the run but do not guarantee neural generation
is bitwise reproducible on another hardware/software stack. Technical
admission does not measure musical preference, listening comfort or clinical
efficacy, and model output is not listening or clinical validation.
