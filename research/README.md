# Music research lab

Three independently runnable experiments using only the Python standard library
and the existing Rust CLI. They do not modify desktop participant settings, start
playback, or update treatment parameters.

| Direction | Testable hypothesis | Prototype | Primary observations |
|---|---|---|---|
| Generation quality screening | Multiple candidates may improve the chance of finding an acceptable source within a fixed generation budget | [Candidate generation and selection](selection.md) | Acceptance rate per independent source, failure reasons, time per accepted source |
| Preference learning | Individual music ratings may help select recipes listeners want to hear again | [Preference recommendations](preferences.md) | Held-out liking scores, repeat listening, explicit discomfort reports |
| Paired controls | Using the same source music can reduce content differences between processing conditions | [Control material preparation](paired.md) | Audio metrics, loudness differences, condition detectability, order sensitivity |

The desktop app can now collect track ratings. Use the [feedback importer](feedback.md)
to prepare one listener's responses for the preference prototype.
The [generation matrix](benchmark.md) compares composer styles and ACE-Step
prompt variants on separate development and holdout seeds, reusing each source
across processing frequencies.

## Getting started

From the repository root:

```bash
cargo build --release --locked
python3 -m unittest discover -s research/tests -v
python3 -m research.selection --help
python3 -m research.preferences --help
python3 -m research.paired --help
python3 -m research.benchmark --help
python3 -m research.feedback --help
```

For ACE-Step, add `--features generation-http` to the build and start the model
service. Keep new experiment audio and raw output in the Git-ignored
`artifacts/audio/` directory. Do not commit participant feedback, configuration
files, or condition mappings; example feedback is synthetic. Use a fresh output
directory for each run to preserve earlier experiments.

## From experiments to research and products

First compare a fixed generation recipe with candidate selection. Group results
by source music and record generation failures and every candidate, including
unselected ones. Stratify by seed, prompt, style, duration, and target frequency
band. Separate training and tuning data from final test sources. Processing
conditions derived from the same music are repeated measurements, not independent
music samples. Evaluate selection performance on held-out sources.

Then assess listening experience: audio quality, sudden sounds, unexpected
vocals, comfort, liking, and willingness to listen again. Technical screening
does not detect these attributes. Preference models optimize music selection
only; ratings must not automatically change notch placement, depth, volume, or
study allocation. Freeze the policy in research mode or specify update rules
in advance.

Initial product work can test whether music creation and listening encourage
continued use, followed by patient studies designed with clinical collaborators.
Any future evaluation of tinnitus changes needs predefined outcomes, follow-up,
controls, and procedures for discomfort events. Technical acceptance rates,
synthetic feedback, and short-term liking are not evidence of treatment efficacy.

## Evidence and sources

[NICE NG155's rationale](https://www.nice.org.uk/guidance/ng155/chapter/Rationale-and-impact)
describes limited evidence for specific sound therapies, so these prototypes
focus on measurable technical and listening experience questions.
[Okamoto et al.'s original trial](https://pubmed.ncbi.nlm.nih.gov/20080545/)
studied tailor-made notched music. This project's generated music and selection
workflow need independent validation; the trial's findings do not transfer
directly. The [ACE-Step 1.5 paper](https://arxiv.org/abs/2602.00744) provides
technical background on music generation. Its generation metrics are not
clinical tinnitus outcomes.

These directions are proposed research hypotheses, not established findings.

## Recorded runs

The [generation matrix results](results-20261021.md) cover 72 executed source
attempts, including 48 ACE-Step outputs, and 288 processing cases. All sources
generated; 282 cases met the full admission rule. The six rejections were drift
sources at 10 kHz. Overtone wording did not improve the observed admission count.
The report includes separate development/holdout results, generation costs and
an executed same-source listening package.

The [2026-10-21 technical record](../artifacts/validation/research-20261021.json)
captures an executed offline smoke test: both 30-second candidates passed, as
did all three processing conditions for the selected source. The loudness range
was approximately `7.60 × 10⁻⁷ LU`. These results come from a single local run;
they are neither clinical data nor a reliable estimate of generation success.
The preference example used synthetic ratings only. That smoke test did not run
ACE-Step.
