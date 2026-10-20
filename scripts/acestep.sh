#!/usr/bin/env bash
# Project-local ACE-Step environment; no system packages or other services changed.
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
runtime_dir="$project_dir/.runtime"
service_dir="$runtime_dir/ACE-Step-1.5"
code_revision="ca1e85fe9430179831e6bc6be790c332190a3866"
model_revision="19671f406d603126926c1b7e2adc169acbcade22"
export HF_HOME="$runtime_dir/huggingface"
export UV_CACHE_DIR="$runtime_dir/uv-cache"
export TMPDIR="$runtime_dir/tmp"
export MPLCONFIGDIR="$runtime_dir/matplotlib"
mkdir -p "$TMPDIR" "$MPLCONFIGDIR"

case "${1:-help}" in
  setup)
    command -v uv >/dev/null || { echo "Install uv, then run this command again." >&2; exit 1; }
    if [[ ! -d "$service_dir/.git" ]]; then
      git clone https://github.com/ace-step/ACE-Step-1.5.git "$service_dir"
      git -C "$service_dir" checkout --detach "$code_revision"
    fi
    if [[ "$(git -C "$service_dir" rev-parse HEAD)" != "$code_revision" ]]; then
      echo "The existing ACE-Step checkout has a different revision. Expected $code_revision; leaving it unchanged." >&2
      exit 1
    fi
    cd "$service_dir"
    uv sync --python "${ACESTEP_PYTHON:-3.12}" --frozen
    .venv/bin/python - "$model_revision" <<'PY'
import json
import sys
from pathlib import Path
from huggingface_hub import snapshot_download
record = {"repo": "ACE-Step/Ace-Step1.5", "revision": sys.argv[1]}
snapshot_download(repo_id=record["repo"], revision=record["revision"], local_dir="checkpoints", max_workers=4)
Path("../model-revision.json").write_text(json.dumps(record, indent=2))
PY
    ;;
  start)
    [[ -x "$service_dir/.venv/bin/python" ]] || { echo "Run bash scripts/acestep.sh setup first." >&2; exit 1; }
    cd "$service_dir"
    .venv/bin/python - "$model_revision" <<'PY'
import json
import sys
from pathlib import Path
weights = [
    "acestep-v15-turbo/model.safetensors", "vae/diffusion_pytorch_model.safetensors",
    "Qwen3-Embedding-0.6B/model.safetensors", "acestep-5Hz-lm-1.7B/model.safetensors",
]
record = Path("../model-revision.json")
ready = record.exists() and json.loads(record.read_text()).get("revision") == sys.argv[1]
if not ready or not all((Path("checkpoints") / weight).is_file() for weight in weights):
    raise SystemExit("Model setup is incomplete. Run bash scripts/acestep.sh setup first.")
PY
    export ACESTEP_INIT_LLM=false
    export ACESTEP_NO_INIT="${ACESTEP_NO_INIT:-false}"
    export ACESTEP_CONFIG_PATH=acestep-v15-turbo
    export ACESTEP_DEVICE="${ACESTEP_DEVICE:-cuda}"
    export ACESTEP_USE_FLASH_ATTENTION=false
    export ACESTEP_OFFLOAD_TO_CPU="${ACESTEP_OFFLOAD_TO_CPU:-false}"
    export ACESTEP_OFFLOAD_DIT_TO_CPU="${ACESTEP_OFFLOAD_DIT_TO_CPU:-false}"
    # Optional allocator limit. Lower-memory GPUs can enable CPU offload above.
    exec .venv/bin/python -u -m acestep.api_server --host 127.0.0.1 --port "${ACESTEP_API_PORT:-8001}"
    ;;
  *)
    echo "Usage: bash scripts/acestep.sh {setup|start}"
    echo "setup: install the pinned Python environment and model in .runtime/"
    echo "start: run the local API in the foreground; Ctrl-C stops it"
    ;;
esac
