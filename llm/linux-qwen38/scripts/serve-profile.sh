#!/usr/bin/env bash
# Launch the Qwen3.8-27B vLLM node under a named profile.
#
#   serve-profile.sh <interactive|concurrent|extended>
#
# Invoked by the systemd units; safe to run by hand for debugging. Profile D
# (quality) is served by llama.cpp, not by this script — see qwen38ctl.
set -euo pipefail

CONF_DIR="${QWEN38_CONF_DIR:-/opt/qwen-vllm/etc}"
PROFILE="${1:-}"

if [ -z "$PROFILE" ]; then
  echo "usage: $(basename "$0") <interactive|concurrent|extended>" >&2
  exit 64
fi

# shellcheck source=../config/common.conf
. "${CONF_DIR}/common.conf"

PROFILE_CONF="${CONF_DIR}/profiles.d/${PROFILE}.conf"
if [ ! -r "$PROFILE_CONF" ]; then
  echo "no such profile: ${PROFILE} (looked for ${PROFILE_CONF})" >&2
  exit 64
fi
# shellcheck source=../config/profiles.d/interactive.conf
. "$PROFILE_CONF"

if [ "${QWEN38_RUNTIME:-vllm}" != "vllm" ]; then
  echo "profile ${PROFILE} is served by ${QWEN38_RUNTIME}, not by this script" >&2
  exit 64
fi

# --- guard: refuse to start into a GPU somebody else already owns ------------
# vLLM otherwise fails minutes into its memory-profiling pass with an error that
# does not name the real cause. The llama.cpp node on this host holds ~23 GiB
# when resident, so this fires often and the message needs to be actionable.
free_mib="$(nvidia-smi --query-gpu=memory.free --format=csv,noheader,nounits | head -1)"
if [ "${free_mib:-0}" -lt "${QWEN38_MIN_FREE_VRAM_MIB}" ]; then
  echo "refusing to start: only ${free_mib} MiB VRAM free, need ${QWEN38_MIN_FREE_VRAM_MIB} MiB" >&2
  nvidia-smi --query-compute-apps=pid,process_name,used_memory --format=csv >&2 || true
  echo "hint: 'qwen-localctl pause' releases the llama.cpp node" >&2
  exit 75   # EX_TEMPFAIL — systemd Restart=on-failure will retry
fi

# Invoking ${QWEN38_VENV}/bin/vllm by absolute path does NOT put the venv's bin
# directory on PATH. FlashInfer JIT-compiles its sampling kernels on first use
# and shells out to `ninja` by name, so without this the engine dies mid-startup
# with FileNotFoundError: 'ninja' -- even though ninja is installed right next
# to the vllm binary being run.
export PATH="${QWEN38_VENV}/bin:${PATH}"
export HF_HOME="${QWEN38_MODELS}"
export VLLM_LOGGING_LEVEL="${VLLM_LOGGING_LEVEL:-INFO}"
# See the FlashInfer sampler workaround in common.conf.
export VLLM_USE_FLASHINFER_SAMPLER="${QWEN38_USE_FLASHINFER_SAMPLER}"
# Optional: see the attention-backend note in common.conf.
if [ -n "${QWEN38_ATTENTION_BACKEND:-}" ]; then
  export VLLM_ATTENTION_BACKEND="${QWEN38_ATTENTION_BACKEND}"
fi
# Keep vLLM's compile cache with the rest of our state instead of in $HOME,
# so ProtectHome=true in the unit does not silently disable it.
export VLLM_CACHE_ROOT="${QWEN38_STATE}/vllm-cache"

args=(
  "${QWEN38_MODEL_REPO}"
  --revision "${QWEN38_MODEL_REVISION}"
  --served-model-name "${QWEN38_SERVED_NAME}"
  --max-model-len "${QWEN38_MAX_MODEL_LEN}"
  --max-num-seqs "${QWEN38_MAX_SEQS}"
  --gpu-memory-utilization "${QWEN38_GPU_MEM_UTIL}"
  --kv-cache-dtype "${QWEN38_KV_DTYPE}"
  --max-num-batched-tokens "${QWEN38_MAX_BATCHED_TOKENS}"
  --host "${QWEN38_HOST}"
  --port "${QWEN38_PORT}"
)

# Frees ~2.5 GiB of CUDA graph memory for KV, at the cost of generation speed.
if [ "${QWEN38_ENFORCE_EAGER:-0}" = "1" ]; then
  args+=(--enforce-eager)
fi

# Text-only worker: refuse image/video items outright. AutoSpec must not
# advertise vision for a worker started this way.
if [ "${QWEN38_MULTIMODAL:-off}" = "off" ]; then
  args+=(--limit-mm-per-prompt '{"image":0,"video":0}')
fi

if [ "${QWEN38_MTP:-off}" = "on" ]; then
  args+=(--speculative-config '{"method":"qwen3_5_mtp","num_speculative_tokens":1}')
fi

if [ -n "${QWEN38_API_KEY:-}" ]; then
  args+=(--api-key "${QWEN38_API_KEY}")
fi

echo "starting profile=${PROFILE} version=${QWEN38_PROFILE_VERSION} ctx=${QWEN38_MAX_MODEL_LEN} seqs=${QWEN38_MAX_SEQS} mtp=${QWEN38_MTP:-off}"
exec "${QWEN38_VENV}/bin/vllm" serve "${args[@]}"
