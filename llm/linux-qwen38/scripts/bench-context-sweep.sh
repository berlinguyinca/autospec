#!/usr/bin/env bash
# Measure how generation and prefill speed change as the context fills up.
#
#   bench-context-sweep.sh <model.gguf> [--kv q8_0|q4_0] [--depths "0 4096 ..."]
#
# Uses llama-bench's -d (depth) flag: it pre-fills the KV cache to N tokens and
# then measures, so the numbers show what you actually get deep into a long
# conversation rather than on an empty context.
#
# Why this matters on THIS model: only 16 of its 64 layers use full attention
# (the other 48 are linear/GDN with constant-size state), so generation speed
# should decay far more slowly with depth than a dense 27B would. This script
# is how that claim gets checked rather than assumed.
set -euo pipefail

MODEL="${1:-}"
if [ -z "$MODEL" ] || [ ! -r "$MODEL" ]; then
  echo "usage: $(basename "$0") <model.gguf> [--kv q8_0|q4_0] [--depths \"...\"]" >&2
  exit 64
fi
shift

# q4_0 KV by default, not q8_0. At 200k tokens q8_0 KV needs ~6.1 GiB on top of
# ~18.5 GiB of weights and does not fit in 24 GiB; q4_0 halves that to ~3.5 GiB.
# This is the same fallback the llama.cpp node already shipped for the same
# reason -- see /opt/qwen-local/etc/qwen-local.conf.
KV="q4_0"
DEPTHS="0 16384 32768 65536 131072 196608"
GEN=128
PP=512
while [ $# -gt 0 ]; do
  case "$1" in
    --kv)     KV="$2"; shift 2 ;;
    --depths) DEPTHS="$2"; shift 2 ;;
    --gen)    GEN="$2"; shift 2 ;;
    *) echo "unknown option: $1" >&2; exit 64 ;;
  esac
done

LLAMA_DIR="${LLAMA_DIR:-/opt/qwen-local/llama.cpp/current}"
BENCH="${LLAMA_DIR}/llama-bench"
[ -x "$BENCH" ] || { echo "llama-bench not found at ${BENCH}" >&2; exit 69; }
export LD_LIBRARY_PATH="${LLAMA_DIR}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

free_mib="$(nvidia-smi --query-gpu=memory.free --format=csv,noheader,nounits | awk '{s+=$1} END{print s+0}')"
if [ "${free_mib}" -lt 20000 ]; then
  echo "need a free GPU: only ${free_mib} MiB available" >&2
  echo "hint: 'qwen38ctl stop'" >&2
  exit 75
fi

echo "model  : $(basename "$MODEL") ($(du -h "$MODEL" | cut -f1))"
echo "kv     : ${KV}"
echo "depths : ${DEPTHS}"
echo

# -d takes a comma-separated list; llama-bench reports pp (prefill) and tg
# (token generation) separately for each depth.
depth_csv="$(echo "$DEPTHS" | tr ' ' ',')"

"$BENCH" \
  -m "$MODEL" \
  -ngl 999 \
  -fa on \
  -ctk "$KV" -ctv "$KV" \
  -p "$PP" \
  -n "$GEN" \
  -d "$depth_csv" \
  -r 2 \
  -o md
