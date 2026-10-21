# Aria sample reproduction

These samples use the official Aria generative finetune and a newly authored
eight-bar piano prompt. The prompt is not a downloaded example or an existing
song. The two sampling seeds are fixed before inference; samples are not rerolled
or selected by perceived quality.

## Pinned inputs

| Component | Source | Revision / version |
| --- | --- | --- |
| Aria implementation | https://github.com/EleutherAI/aria | `c2f67bc90b49335543c0249fe00fb84a9337faf8` |
| Aria tokenizer | https://github.com/EleutherAI/aria-utils | `7c815755c6889a784b6a1b3343e9e8daa3e7d128` |
| Generative checkpoint | https://huggingface.co/loubb/aria-medium-base | `5a1ef36c5007ee10fbe57d61d96725a7244bd2b2`, file `model-gen.safetensors` |
| Python | Existing project-local ACE environment's interpreter | `3.12.13` |
| PyTorch | Existing project-local ACE environment, read through `.pth` | `2.10.0+cu128` |
| Additional local packages | PyPI, installed only into the new Aria environment | `mido==1.3.3`, `jsonlines==4.0.0`, `attrs==26.1.0`, `packaging==26.3` |

The standalone `loubb/aria-medium-gen` URL linked from the GitHub README returned
HTTP 401 during this run. The official author's
[base-model card](https://huggingface.co/loubb/aria-medium-base) explicitly links
the generative variant as `model-gen.safetensors` in that public repository.
That official file is used. No third-party checkpoint mirror is involved.

The complete checkpoint's verified SHA-256 is
`5e6b07ee2680f0044deea30da613d89b12e3e30d012d1812b388d6b8d8306c90`.
During the recorded run, Xet stalled on two chunks. A separate snapshot was
repaired from official HTTPS byte ranges, including overlapping boundary bytes,
then checked against the official LFS SHA-256 before inference. The stalled
writer was stopped. The command below disables Xet to avoid that transport issue.

The source and model card identify the model/tooling license as Apache-2.0.

## Isolated setup

Run from the project root. The existing ACE environment must be available; it is
not modified. Only its installed Python libraries are made visible to Aria.
The source directories are ignored runtime dependencies, not vendored project
code. If a checkout already exists, inspect it before replacing or changing it.

```bash
git clone https://github.com/EleutherAI/aria.git .runtime/aria
git -C .runtime/aria checkout --detach c2f67bc90b49335543c0249fe00fb84a9337faf8
git clone https://github.com/EleutherAI/aria-utils.git .runtime/aria-utils
git -C .runtime/aria-utils checkout --detach 7c815755c6889a784b6a1b3343e9e8daa3e7d128
.runtime/ACE-Step-1.5/.venv/bin/python -m venv --without-pip .runtime/aria-venv
.runtime/aria-venv/bin/python - <<'PY'
from pathlib import Path
site = next(Path('.runtime/aria-venv/lib').glob('python*/site-packages'))
paths = [
    Path('.runtime/ACE-Step-1.5/.venv/lib/python3.12/site-packages').resolve(),
    Path('.runtime/aria').resolve(),
    Path('.runtime/aria-utils').resolve(),
]
(site / 'aria_runtime.pth').write_text('\n'.join(map(str, paths)) + '\n')
PY
uv pip install --python .runtime/aria-venv/bin/python \
  mido==1.3.3 jsonlines==4.0.0 attrs==26.1.0 packaging==26.3
```

Download only the required official checkpoint and metadata at the pinned commit:

```bash
HF_HUB_DISABLE_XET=1 .runtime/aria-venv/bin/python - <<'PY'
from pathlib import Path
import json
from huggingface_hub import HfApi, hf_hub_download

repo = 'loubb/aria-medium-base'
revision = '5a1ef36c5007ee10fbe57d61d96725a7244bd2b2'
destination = Path('.runtime/aria-checkpoint')
destination.mkdir(parents=True, exist_ok=True)
info = HfApi(token=False).model_info(repo, revision=revision, files_metadata=True)
(destination / 'provenance.json').write_text(json.dumps({
    'repo_id': info.id,
    'revision': info.sha,
    'files': [(file.rfilename, file.size) for file in info.siblings],
}, indent=2) + '\n')
for filename in ['README.md', 'config.json', 'model-gen.safetensors']:
    hf_hub_download(repo, filename, revision=revision,
                    local_dir=str(destination), token=False)
PY
```

## Generate and render

```bash
.runtime/aria-venv/bin/python research/listening/aria_generate.py \
  --seeds 21021 21022 --max-new-tokens 2048
```

The runner uses one sample at a time, BF16 model weights, temperature `0.90`,
minimum-probability sampling `0.035`, no compilation, and a 16 GiB GPU allocator
limit. It stops if required model parameters are missing. The original prompt
contains 64 notes over 30.64 seconds in D major, with intentional phrasing,
different left/right-hand velocities, and modest rubato.

Outputs are written under `artifacts/audio/blind-20261021/aria/`:

- `original-eight-bar-seed.mid` and `seed.json`: original input and exact notes.
- `aria-seed-21021-full.mid`, `aria-seed-21022-full.mid`: complete model outputs,
  including the prompt, retained for auditing.
- `aria-seed-21021-continuation.mid`, `aria-seed-21022-continuation.mid`: up to
  60 seconds beginning after the original prompt. The crop excludes all original
  input notes; generated pitches, velocities, and onset times are preserved.
- `generation.json`: checkpoint/code/tokenizer revisions, SHA-256 hashes,
  exact sampling settings, per-sample GPU times/memory, and crop boundaries.
- `validation.json`: MIDI note-on/off balance, acoustic-piano program checks,
  duration checks, and proof that a refused rerun did not change the report.
- `generation-error.json`: written on failure; no replacement sample is generated.

Render each generated-only MIDI with the common Salamander/sfizz pipeline:

```bash
python3 research/listening/render_piano.py \
  artifacts/audio/blind-20261021/aria/aria-seed-21021-continuation.mid \
  artifacts/audio/blind-20261021/render/aria-seed-21021-continuation.wav
python3 research/listening/render_piano.py \
  artifacts/audio/blind-20261021/aria/aria-seed-21022-continuation.mid \
  artifacts/audio/blind-20261021/render/aria-seed-21022-continuation.wav
```

The runner refuses to overwrite an existing generation report or generated MIDI.
An identical existing authored seed can be reused without rewriting it.

The blind demo uses the first 30 seconds of the generated-only audio, with its
common loudness-matching and boundary fades. No tinnitus-frequency filter,
denoising, note correction, or extra instrumental layer is applied here.

Aria generates note events rather than a waveform. It cannot guarantee musical
quality or adherence to a text instruction; the blind comparison is intended to
evaluate the resulting music. Numerical MIDI validation is not a listening test.
