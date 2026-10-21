# Stable Audio 3 sample reproduction

This route uses Stability AI's official Small Music and Medium checkpoints to
generate audio directly. It does not use the MIDI piano renderer. The two models
receive the same solo-piano prompt and the same two seeds, fixed before inference.

## Pinned inputs

| Component | Revision |
| --- | --- |
| [Official implementation](https://github.com/Stability-AI/stable-audio-3) | `77e0398c26f7d22d801ec1bb036307e8b231f741` |
| [Small Music](https://huggingface.co/stabilityai/stable-audio-3-small-music) | `8bdfb53c6b0fbc62113cd571fd37373efe15fa14` |
| [Medium](https://huggingface.co/stabilityai/stable-audio-3-medium) | `d4d3579395a8ac0df4ab13b5b1d7138ce7290f81` |

Model access terms were accepted through the user's account. The downloader uses
the authorized read token and original Hugging Face repositories. It retains the
model license, Gemma license, configuration, bundled T5Gemma encoder/tokenizer,
and file hashes. Every large file is verified against its official LFS SHA-256.
Incomplete transfers remain `.partial` files and resume from their byte offset.
Credentials are loaded in-process from the ignored project access file; they do
not appear in command arguments or output.

## Isolated runtime

The source is checked out under `.runtime/stable-audio-3`. A separate Python 3.12
environment, `.runtime/stable-audio-venv`, reads the existing ACE environment's
PyTorch 2.10.0+cu128 installation through a `.pth` entry. It does not modify ACE.
PyTorch differs from the upstream 2.7.1 pin to use the existing Blackwell-compatible
CUDA runtime. Other packages are installed into the new environment:

```sh
.runtime/ACE-Step-1.5/.venv/bin/python -m venv --without-pip .runtime/stable-audio-venv
.runtime/stable-audio-venv/bin/python - <<'PY'
from pathlib import Path
site = next(Path('.runtime/stable-audio-venv/lib').glob('python*/site-packages'))
paths = [Path('.runtime/ACE-Step-1.5/.venv/lib/python3.12/site-packages').resolve(),
         Path('.runtime/stable-audio-3').resolve()]
(site / 'stable_audio_runtime.pth').write_text('\n'.join(map(str, paths)) + '\n')
PY
uv pip install --python .runtime/stable-audio-venv/bin/python \
  transformers==5.8.0 huggingface-hub==1.7.1 tqdm==4.67.3 packaging==26.3 \
  einops-exts==0.0.4 'httpx[socks]' 'requests[socks]'
```

The upstream attention code has built-in Flex Attention and SDPA fallbacks when
Flash Attention is absent. Those original paths are retained; model architecture
and weights are unchanged. Full dependency versions and the attention path are
recorded alongside each actual generation.

## Download and generate

```sh
.runtime/stable-audio-venv/bin/python research/listening/stable_audio_setup.py \
  --proxy socks5h://127.0.0.1:1080
.runtime/stable-audio-venv/bin/python research/listening/stable_audio_generate.py --model small-music
.runtime/stable-audio-venv/bin/python research/listening/stable_audio_generate.py --model medium
```

The proxy is specific to this server; omit it on a host with working direct
access. Normal TLS verification remains enabled. Generation runs offline from
the verified local files. The only configuration change is replacing the encoder
repository location with its pinned local directory. Strict tensor loading
rejects missing, unexpected, or incompatible checkpoint tensors.

Seeds are `210731` and `210732`; each source is 60 seconds of 44.1 kHz stereo
audio, generated with the official post-trained default of 8 steps and CFG 1.
Negative prompting is not used because the post-trained models ignore it. The
common prompt requests a restrained, lyrical acoustic piano solo at 64 BPM in D
major. Unchunked decoding avoids introducing chunk joins. Float WAV files retain
the raw generated samples before the common blind-set excerpt/level processing.

Outputs and complete records live under
`artifacts/audio/blind-20261021/stable-audio/{small-music,medium}/`. Existing
outputs are not overwritten. The listening builder takes the first 30 seconds
and applies the same edge fades and gain matching as the other sources.
