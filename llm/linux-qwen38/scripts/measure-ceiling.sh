#!/usr/bin/env bash
# Measure the real context ceiling of a profile on this GPU, and prove it.
#
#   measure-ceiling.sh <profile> [--ctx-only]
#
# Reads the profile's own settings, so it measures the configuration that will
# actually be served rather than some default. Two stages:
#
#   probe   ask for the full training context; vLLM's refusal states an
#           "estimated maximum model length". That estimate depends on what you
#           asked for, so it is iterated to a fixed point.
#
#   verify  restart at the candidate and run a needle-in-haystack prompt filling
#           ~90% of it. This stage is the whole point, and it must use a LONG
#           prompt. Three different numbers looked like the ceiling on this host
#           and only the smallest was real:
#             66,446  "GPU KV cache size" -- an aggregate pool, not a per-request
#                     limit; not comparable to max_model_len at all.
#            109,760  the estimate from probing at 262144. Starts, serves a
#                     6-token request, then OOMs on a ~70k prompt.
#             39,200  the fixed-point estimate. Survives a long prompt.
#           "Allocates", "starts", and "works at length" are three different
#           claims. Only the third is reported as verified.
set -euo pipefail

CONF_DIR="${QWEN38_CONF_DIR:-/opt/qwen-vllm/etc}"
PROFILE="${1:-}"
CTX_ONLY=0
# if/then, never `[ test ] && action`: under `set -e` a one-sided && whose test
# is false makes the whole statement non-zero and kills the script.
if [ "${2:-}" = "--ctx-only" ]; then CTX_ONLY=1; fi

if [ -z "$PROFILE" ]; then
  echo "usage: $(basename "$0") <profile> [--ctx-only]" >&2
  exit 64
fi

# shellcheck source=../config/common.conf
. "${CONF_DIR}/common.conf"
PROFILE_CONF="${CONF_DIR}/profiles.d/${PROFILE}.conf"
[ -r "$PROFILE_CONF" ] || { echo "no such profile: ${PROFILE}" >&2; exit 64; }
# shellcheck disable=SC1090
. "$PROFILE_CONF"

[ "${QWEN38_RUNTIME:-vllm}" = "vllm" ] || {
  echo "profile ${PROFILE} is not served by vLLM" >&2; exit 64; }

free_mib="$(nvidia-smi --query-gpu=memory.free --format=csv,noheader,nounits | awk '{s+=$1} END{print s+0}')"
if [ "${free_mib}" -lt "${QWEN38_MIN_FREE_VRAM_MIB}" ]; then
  echo "need a free GPU: only ${free_mib} MiB available" >&2
  echo "hint: 'qwen38ctl stop' or 'qwen-localctl pause'" >&2
  exit 75
fi

# See serve-profile.sh: FlashInfer's JIT shells out to `ninja` by name.
export PATH="${QWEN38_VENV}/bin:${PATH}"
export HF_HOME="${QWEN38_MODELS}"
# The compile cache is set below, after we know whether the state tree is
# writable by whoever is running this.
# See the FlashInfer sampler workaround in common.conf.
export VLLM_USE_FLASHINFER_SAMPLER="${QWEN38_USE_FLASHINFER_SAMPLER}"
# Optional: see the attention-backend note in common.conf.
if [ -n "${QWEN38_ATTENTION_BACKEND:-}" ]; then
  export VLLM_ATTENTION_BACKEND="${QWEN38_ATTENTION_BACKEND}"
fi

# Do not inherit the caller's cwd. Run under `sudo -u qwen-vllm` from a home
# directory and vLLM dies with
#   PermissionError: [Errno 13] Permission denied: '/home/<you>/...'
# because the service account cannot read it. The state dir always works.
cd "${QWEN38_STATE}" 2>/dev/null || cd /tmp

PROBE_CTX=262144   # the model's n_ctx_train; vLLM validates against this
util="${QWEN38_GPU_MEM_UTIL}"   # descends on OOM; see the verify loop
HELPER="$(dirname "$(readlink -f "$0")")/long-prompt-probe.py"
if [ ! -r "$HELPER" ]; then HELPER="/opt/qwen-vllm/bin/long-prompt-probe.py"; fi
TAG="${PROFILE}"
BASE="http://${QWEN38_HOST}:${QWEN38_PORT}"

# The state tree belongs to the service account, so an operator running this by
# hand cannot write ${QWEN38_LOGS}. Without this check the very first redirect
# fails and `set -e` kills the script before it prints anything at all -- the
# failure looks like "it did nothing", which is the worst possible symptom.
LOGDIR="${QWEN38_LOGS}"
export VLLM_CACHE_ROOT="${QWEN38_STATE}/vllm-cache"
mkdir -p "$LOGDIR" 2>/dev/null || true
if ! { : > "${LOGDIR}/.write-probe"; } 2>/dev/null; then
  # Both the logs AND the torch.compile cache live under the service account's
  # state tree. Redirecting only the logs is not enough: measuring a context the
  # cache has not seen before makes vLLM create a new compile-cache entry, which
  # fails with PermissionError deep inside engine startup and looks like an
  # engine crash rather than a permissions problem.
  fallback="${TMPDIR:-/tmp}/qwen38-measure-$(id -un)"
  LOGDIR="${fallback}/logs"
  export VLLM_CACHE_ROOT="${fallback}/vllm-cache"
  mkdir -p "$LOGDIR" "$VLLM_CACHE_ROOT"
  echo "note: ${QWEN38_STATE} is not writable by $(id -un)."
  echo "      logs  -> ${LOGDIR}"
  echo "      cache -> ${VLLM_CACHE_ROOT} (cold: expect a slow first compile)"
  echo "      run under 'sudo -u qwen-vllm' to reuse the service's warm cache."
else
  rm -f "${LOGDIR}/.write-probe"
fi

# Every knob that affects the memory profile is taken from the profile, so the
# measured pool is the pool that profile will actually get.
build_args() {
  local ctx="$1"
  args=(
    "${QWEN38_MODEL_REPO}" --revision "${QWEN38_MODEL_REVISION}"
    --served-model-name "${QWEN38_SERVED_NAME}"
    --max-model-len "$ctx"
    --max-num-seqs "${QWEN38_MAX_SEQS}"
    --gpu-memory-utilization "${util}"
    --kv-cache-dtype "${QWEN38_KV_DTYPE}"
    --max-num-batched-tokens "${QWEN38_MAX_BATCHED_TOKENS}"
    --host "${QWEN38_HOST}" --port "${QWEN38_PORT}"
  )
  # if/then/fi, not `[ test ] && append`. With `set -e`, a false test makes the
  # function return non-zero and the caller dies -- which is exactly what
  # happened for every profile with CUDA graphs enabled: the script printed its
  # probe header and vanished without ever launching vLLM.
  if [ "${QWEN38_MULTIMODAL:-off}" = "off" ]; then
    args+=(--limit-mm-per-prompt '{"image":0,"video":0}')
  fi
  if [ "${QWEN38_ENFORCE_EAGER:-0}" = "1" ]; then
    args+=(--enforce-eager)
  fi
  if [ "${QWEN38_MTP:-off}" = "on" ]; then
    args+=(--speculative-config '{"method":"qwen3_5_mtp","num_speculative_tokens":1}')
  fi
}

srv=""
cleanup() { if [ -n "$srv" ]; then kill "$srv" 2>/dev/null || true; fi; return 0; }
trap cleanup EXIT

start_and_wait() {   # $1 = log file; returns 0 once healthy
  local log="$1" waited=0
  "${QWEN38_VENV}/bin/vllm" serve "${args[@]}" > "$log" 2>&1 &
  srv=$!
  until curl -sS --fail --max-time 5 "${BASE}/health" >/dev/null 2>&1; do
    if ! kill -0 "$srv" 2>/dev/null; then
      echo "server exited during startup; last lines:" >&2; tail -25 "$log" >&2; return 1
    fi
      if [ "$waited" -ge 1200 ]; then echo "timed out after ${waited}s" >&2; return 1; fi
    sleep 5; waited=$((waited + 5))
  done
  echo "healthy after ${waited}s"
}

# ---- probe -----------------------------------------------------------------
# Ask for the model's full training context. vLLM refuses and, in refusing,
# states the answer outright:
#
#   "... 2.06 GiB KV cache is needed, which is larger than the available KV
#    cache memory (1.36 GiB). Based on the available memory, the estimated
#    maximum model length is 39200."
#
# That estimate is the number we want. Do NOT use "GPU KV cache size: N tokens"
# instead -- it is an aggregate pool figure and is NOT comparable to
# max_model_len. On this model the pool reads 66,446 tokens while a single
# sequence tops out at 39,200, because one long sequence also consumes a mamba
# state page per block. Treating the pool as the ceiling overshoots by ~70%.
#
# The estimate must be iterated to a FIXED POINT. vLLM's available-KV figure
# itself depends on the max_model_len you asked for -- measured on this host:
#
#     asked 262144 -> available 3.51 GiB -> estimate 109760
#     asked 109760 -> available 1.36 GiB -> estimate  39200
#     asked  62720 -> available 1.36 GiB -> estimate  39200
#
# So a single probe reports a ceiling that is not valid at its own answer.
# Re-probe at each estimate until it stops moving; 39200 is the fixed point here.
probe_log="${LOGDIR}/measure-${TAG}.probe.log"
echo "=== probe: ${PROFILE}, iterating to a fixed point ==="
ask="$PROBE_CTX"
usable=""
for _ in 1 2 3 4 5; do
  build_args "$ask"
  if start_and_wait "$probe_log"; then
    usable="$ask"
    kill "$srv" 2>/dev/null; srv=""; sleep 5
    echo "  asked ${ask} -> starts; this is the ceiling"
    break
  fi
  est="$(grep -aoE 'estimated maximum model length is [0-9]+' "$probe_log" \
         | tail -1 | grep -oE '[0-9]+$' || true)"
  if [ -z "$est" ]; then
    echo "could not read an estimated maximum model length from ${probe_log}" >&2
    tail -20 "$probe_log" >&2
    exit 1
  fi
  echo "  asked ${ask} -> estimate ${est}"
  if [ "$est" = "$ask" ]; then usable="$est"; break; fi
  ask="$est"
  usable="$est"
  sleep 5
done

echo
echo "vllm estimate : ${usable} tokens"
echo "configured    : ${QWEN38_MAX_MODEL_LEN} tokens"
if [ "$CTX_ONLY" -eq 1 ]; then exit 0; fi

# ---- verify, descending until a long prompt actually succeeds ---------------
# The fixed-point estimate is an upper bound, not an answer: a context can start
# and still fail a prompt that fills it (measured: 109760 boots, serves a short
# request, then 500s on a long one). So descend until a long prompt passes,
# rather than reporting the first value that merely boots.
verify_log="${LOGDIR}/measure-${TAG}.verify.log"
block=1568
verified=""
tokens=""
used_vram=""

for _ in 1 2 3 4; do
  echo
  echo "=== verify: long prompt at ctx=${usable} ==="
  build_args "$usable"
  if ! start_and_wait "$verify_log"; then
    usable=$(( usable * 70 / 100 / block * block ))
    if [ "$usable" -lt "$block" ]; then break; fi
    echo "  did not start; trying ${usable}"
    continue
  fi

  out="$("${QWEN38_VENV}/bin/python" "${HELPER}" "$BASE" "$QWEN38_SERVED_NAME" "$usable")"
  used_vram="$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | awk '{s+=$1} END{print s+0}')"
  echo "  long-prompt probe: ${out}"
  kill "$srv" 2>/dev/null; srv=""; sleep 5

  case "$out" in
    CEILING_OK*) verified="$usable"; tokens="${out#CEILING_OK }"; break ;;
  esac

  # LOWER UTILISATION, not context. vLLM sizes the KV pool from
  # gpu_memory_utilization, NOT from max_model_len -- so shrinking the context
  # frees no VRAM at all and the same OOM repeats at every size. Measured: the
  # gated-delta-rule prefill kernel died with 19 MiB free at ctx 109760, 76832,
  # 53312 and 36064 alike:
  #   chunk_gated_delta_rule_fwd_h -> torch.OutOfMemoryError: Tried to allocate
  #   20.00 MiB ... 19.62 MiB is free
  # Both FlashInfer's workspace and the GDN prefill kernel allocate working
  # memory outside the profiled budget, so headroom is the only real lever.
  util="$(python3 -c "print(f'{max(0.86, ${util} - 0.02):.2f}')")"
  echo "  failed at length; lowering utilisation to ${util}"
done

echo
if [ -n "$verified" ]; then
  echo "VERIFIED  profile=${PROFILE} ctx=${verified} (long prompt ${tokens} tokens) vram=${used_vram}MiB"
  echo "configured in profile: ${QWEN38_MAX_MODEL_LEN}"
else
  echo "NO VERIFIED CEILING FOUND for ${PROFILE}" >&2
  tail -20 "$verify_log" >&2
  exit 1
fi
