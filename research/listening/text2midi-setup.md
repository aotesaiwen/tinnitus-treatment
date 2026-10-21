# Text2midi local blind-test setup

Two fresh generations were completed on 2026-10-21 using the official model. Both produced valid MIDI longer than 30 seconds, but both failed the requested solo-piano instrumentation. The original outputs and token streams are preserved; the generation runner does not rewrite notes, remove percussion, change instruments, or adjust tempo.

## Pinned inputs

| Input | Revision / verification |
| --- | --- |
| [Official source](https://github.com/AMAAI-Lab/Text2midi) | `f3245ee402b29ba00b86c6eb9813032f3f05954f` |
| [Official checkpoint and REMI tokenizer](https://huggingface.co/amaai-lab/text2midi) | `9bc91479c4a9b08a9a28a7dcfb3f1a56ee2ded36` |
| `pytorch_model.bin` | 901,867,614 bytes; SHA-256 `d94e8d7dda9207ed734268933629d4a4b7a1a6ce84c77299f610d3470448bfc1` |
| `vocab_remi.pkl` | SHA-256 `877d4511d6b9d5eea1c706199fe13a0de3d984c8f5d09c75d727ffe7f6f54ee6` |
| [FLAN-T5 configuration and text tokenizer](https://huggingface.co/google/flan-t5-base) | `7bcac572ce56db69c1ea7c8af255c5d7c9672fc2` |

The Text2midi checkpoint contains the complete encoder and decoder: 406 state-dict entries, including 112 encoder entries. The runner constructs T5 from its pinned configuration and requires a strict checkpoint load. It therefore does not need a separate download of FLAN-T5 model weights. No randomly initialized encoder parameters are left in the model.

## Environment

The run used Python 3.12.13 and PyTorch 2.10.0+cu128 on an RTX PRO 6000 Blackwell. A separate environment installs Text2midi-specific dependencies. A `.pth` file references the existing ACE-Step environment for read-only reuse of PyTorch and common packages; nothing is installed into or modified in that environment.

From the project root, with the existing ACE-Step runtime available:

```bash
uv venv --python .runtime/ACE-Step-1.5/.venv/bin/python .runtime/text2midi-env
.runtime/text2midi-env/bin/python - <<'PY'
from pathlib import Path
import sysconfig
shared = Path('.runtime/ACE-Step-1.5/.venv/lib/python3.12/site-packages').resolve()
(Path(sysconfig.get_paths()['purelib']) / 'reuse_cached_torch.pth').write_text(str(shared) + '\n')
PY
uv pip install --python .runtime/text2midi-env/bin/python \
  --default-index https://pypi.org/simple --no-deps \
  transformers==4.47.0 tokenizers==0.21.0 miditok==3.0.3 symusic==0.5.5 \
  st-moe-pytorch==0.1.8 jsonlines==4.0.0 colt5-attention==0.11.1 \
  local-attention==1.9.15 beartype==0.19.0 pysmartdl==1.3.4 \
  sentencepiece==0.2.0 mido==1.3.3 platformdirs==4.3.6
```

Relevant reused versions were `huggingface-hub==0.36.0`, `accelerate==1.12.0`, `einops==0.8.2`, and `numpy==2.3.5`. The configured Tsinghua package mirror timed out; installing the same pinned packages from official PyPI succeeded. A stalled Hugging Face Xet checkpoint transfer was restarted with ordinary HTTPS. The final checkpoint hash matches its official LFS object hash.

## Download the pinned inputs

These commands describe a fresh setup; do not clone over an existing runtime directory.

```bash
git clone https://github.com/AMAAI-Lab/Text2midi.git .runtime/text2midi
git -C .runtime/text2midi checkout f3245ee402b29ba00b86c6eb9813032f3f05954f
HF_HUB_DISABLE_XET=1 .runtime/text2midi-env/bin/python - <<'PY'
from pathlib import Path
import hashlib
import json
from huggingface_hub import snapshot_download

models = [
    ('amaai-lab/text2midi', '9bc91479c4a9b08a9a28a7dcfb3f1a56ee2ded36',
     '.runtime/text2midi-weights',
     ['pytorch_model.bin', 'vocab_remi.pkl', 'README.md', 'transformer_model.py']),
    ('google/flan-t5-base', '7bcac572ce56db69c1ea7c8af255c5d7c9672fc2',
     '.runtime/text2midi-flan-t5-base',
     ['config.json', 'spiece.model', 'tokenizer.json', 'tokenizer_config.json', 'special_tokens_map.json']),
]
for repo, revision, directory, files in models:
    root = Path(directory)
    snapshot_download(repo, revision=revision, local_dir=root, allow_patterns=files)
    record = {'repo': repo, 'revision': revision}
    checkpoint = root / 'pytorch_model.bin'
    if checkpoint.exists():
        with checkpoint.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        assert digest == 'd94e8d7dda9207ed734268933629d4a4b7a1a6ce84c77299f610d3470448bfc1'
        record.update(checkpoint_sha256=digest, checkpoint_bytes=checkpoint.stat().st_size)
    (root / 'revision.json').write_text(json.dumps(record, indent=2) + '\n')
PY
```

## Generate

```bash
HF_HUB_OFFLINE=1 .runtime/text2midi-env/bin/python -u \
  research/listening/text2midi_generate.py \
  --seeds 210641 210682 --max-tokens 2000 --temperature 0.9
```

The runner uses the official `Transformer.generate` sampler in float32 under `torch.inference_mode()`, with a 20 GiB per-process GPU allocation cap. It does not constrain instrument tokens or alter sampling. It refuses to overwrite existing generated MIDI; use `--output` with a new directory for another run. Exact prompts, seeds, revisions, timings, and MIDI analysis are in `artifacts/audio/blind-20261021/text2midi/manifest.json`.

## Results and implementation check

| Seed | Generated duration | Generation time | Note events | Requested tempo | Actual tempo |
| --- | ---: | ---: | --- | ---: | --- |
| 210641 | 37.686 s | 52.989 s | 316 pitched-channel notes + 148 channel-9 percussion notes | 64 BPM | 87.42–128.06 BPM |
| 210682 | 34.911 s | 53.508 s | 396 pitched-channel notes + 76 channel-9 percussion notes | 68 BPM | 67.10 BPM |

The first output contains eight non-drum programs, including distorted guitar, strings, bass, tubular bells, and saxophone. The second contains seven non-drum programs; its piano program has only one note, while distorted guitar has 278. Both include percussion despite prompts requesting piano alone. “Pitched-channel notes” refers to MIDI channels other than channel 9 and does not assert that every selected GM instrument is acoustically pitched.

To check that this was not caused by using a mismatched implementation, the checkpoint's bundled `transformer_model.py` was compared with the pinned GitHub source. All seven shared model classes, including `Transformer.generate`, have identical abstract syntax trees. The official GitHub and Hugging Face REMI tokenizer files are byte-identical. The full model checkpoint passed strict state-dict loading and its SHA-256 verification. No architecture, checkpoint, or tokenizer mismatch was found.

The model's solo-piano prompt adherence failed in these two fixed samples. Separate listening arrangements were subsequently created in `artifacts/audio/blind-20261021/render/`: channel-9 events were removed and all remaining program changes mapped to acoustic grand piano. The original MIDI remains unchanged. An independent `mido` check confirmed that all remaining note on/off events, velocities, controls, tempo events, time signatures, and key signatures match the original at the same absolute ticks. These checks are saved in `text2midi/arrangement-verification.json`.

The blind-test reveal must identify these as piano arrangements, not native solo-piano model outputs. No additional seeds were generated to hide the failures. The generation process exited successfully and released its GPU allocation.
