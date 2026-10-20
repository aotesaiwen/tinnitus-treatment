# Personal music preference exploration

This offline prototype recommends one candidate from a predefined recipe catalog
for experiments on what to generate next. It uses only the Python 3 standard
library. Both example participants and all their ratings are **synthetic data**.

Run from the repository root:

```bash
python3 -m research.preferences --input research/examples/preferences.json --participant synthetic-alice --seed 42
python3 -m research.preferences --input research/examples/preferences.json --participant synthetic-bob --seed 42
python3 -m unittest discover -s research/tests -p 'test_preferences.py' -v
```

Output includes the full selected recipe, candidate scores, the participant's
feedback count, exclusion reasons, and random seed. The same input and seed
produce the same output with the same Python implementation and version.
Candidates are sorted by ID; feedback is read in input order. Preserve the
original JSON and runtime environment for reproducibility. `--seed` controls
recommendation exploration, not audio generation.

## Input

- `catalog`: a nonempty recipe array. Each entry requires a unique `recipe_id`,
  `provider` (`composer` / `ace_step`), `style` (`drift` / `focus` / `unwind`),
  integer `tempo_bpm` (45–100), integer `duration_s` (30–300), and `prompt`
  (up to 500 characters). The `composer` does not support text prompts, so its
  `prompt` must be empty or whitespace only; `ace_step` accepts nonempty prompts.
  Keep IDs bound to recipe contents and assign a new ID when changing a recipe.
- `feedback`: may be empty. Each entry contains `participant_id`, a `recipe_id`
  present in the catalog, a finite numeric `rating` from 1–5, and a boolean
  `discomfort`. Participant IDs match desktop profiles exactly, including any
  surrounding whitespace; empty or whitespace-only IDs are invalid.
- `synthetic`: optional boolean flag; `true` in the example.

Each feedback entry counts as a separate rating, with no deduplication by recipe.
Callers must exclude duplicate event imports. Unknown fields, duplicate recipe
IDs, duplicate JSON keys, unknown recipe references, and invalid ratings cause
an error and CLI exit code `2`. The entire file is validated before computing
recommendations using only the selected participant's feedback.

## Scoring

For each recipe without a discomfort report from this participant, transform
ratings to `x = (rating - 1) / 4`, then set `alpha = 2 + Σx` and
`beta = 2 + Σ(1 - x)`. The `regularized_score = alpha / (alpha + beta)` shrinks
toward 0.5 when data is sparse. Using the specified seed, draw from each recipe's
`Beta(alpha, beta)` distribution and select the largest draw. The highest-rated
recipe therefore does not always win, and unrated recipes retain a chance of
selection. Fractional counts provide a heuristic update; these distributions
are not validated clinical posteriors or confidence intervals.

Any `discomfort=true` report excludes that recipe for that participant; later high
ratings do not undo the exclusion. Other participants' ratings and discomfort
reports do not affect the result. Excluding every candidate produces an error.

## Research and product boundaries

Preference exploration does not establish tinnitus improvement or causal
effects. The prototype does not model correlation between repeated ratings,
selection bias, expectation effects, or changing preferences. Comparisons of
fixed recipes with adaptive recommendations need a separate preregistered
evaluation, independent outcome measures, and selection probability logs.
A sampled score is not the recipe's selection probability and cannot be used
directly for offline causal estimation.

The prototype writes JSON to standard output. It does not store participant
profiles, call models, or start playback. Callers supply a catalog of available
recipes; the program does not check ACE-Step service availability. It does not
read or adjust tinnitus frequency, notch settings, gain, or treatment allocation.
Product integration should retain the existing generation, audio validation,
processing with current settings, and user-initiated playback workflow.
Preference scores and an absence of reported discomfort do not establish
playback safety.
