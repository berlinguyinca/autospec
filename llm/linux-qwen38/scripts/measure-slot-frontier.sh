#!/usr/bin/env bash
# For each slot count, find the largest shared context pool that still loads.
#
#   measure-slot-frontier.sh <preset-section> <parallel-list> [start-ctx] [step]
#
# Two VRAM costs move in opposite directions and both must fit:
#
#   the KV pool      grows with --ctx-size, and is shared across slots when
#                    kv-unified is on
#   compute buffers  grow with --parallel, because every slot needs its own
#                    activation and logit space for a batched decode step
#
# So "more seats" is paid for in context, not in memory you can conjure. This
# script measures the exchange rate instead of guessing it. It only proves the
# configuration LOADS -- serving a prompt that actually fills the pool is a
# separate claim, checked afterwards with long-prompt-probe.py.
set -uo pipefail

PRESETS="${QWEN38_ROUTER_PRESETS:-/opt/qwen-vllm/etc/router-presets.ini}"
UNIT="${QWEN38_UNIT:-autospec-qwen38@router.service}"
BASE="${QWEN38_BASE:-http://127.0.0.1:8080}"

SECTION="${1:-}"
PARALLEL_LIST="${2:-1,2,4,6,8}"
START_CTX="${3:-196608}"
STEP="${4:-16384}"

if [ -z "$SECTION" ]; then
  echo "usage: $(basename "$0") <preset-section> [parallel-list] [start-ctx] [step]" >&2
  exit 64
fi

BACKUP="$(mktemp)"
cp "$PRESETS" "$BACKUP"
# Inline, not a RETURN/EXIT trap that assumes a frame: this must restore the
# operator's serving configuration even when a probe wedges the unit.
restore() {
  sudo cp "$BACKUP" "$PRESETS"
  sudo systemctl restart "$UNIT" || true
  rm -f "$BACKUP"
}
trap 'restore' EXIT INT TERM

# Rewrite one key inside one INI section, leaving every other section alone.
set_key() {
  sudo python3 - "$PRESETS" "$SECTION" "$1" "$2" <<'PY'
import re, sys
path, section, key, value = sys.argv[1:5]
text = open(path).read()
head = f"[{section}]"
i = text.index(head)
j = text.find("\n[", i + 1)
j = len(text) if j == -1 else j
body = text[i:j]
pat = re.compile(rf"^{re.escape(key)} *=.*$", re.M)
body = pat.sub(f"{key} = {value}", body) if pat.search(body) \
    else body.rstrip("\n") + f"\n{key} = {value}\n"
open(path, "w").write(text[:i] + body + text[j:])
PY
}

# A load either succeeds or it does not; the router reports both as HTTP, so ask
# it for one token rather than parsing the log.
try_load() {
  sudo systemctl restart "$UNIT" >/dev/null 2>&1
  sleep 4
  local out
  out="$(curl -s -m 600 "${BASE}/v1/chat/completions" \
        -H 'Content-Type: application/json' \
        -d "{\"model\":\"${SECTION}\",\"max_tokens\":4,\"temperature\":0,\
\"chat_template_kwargs\":{\"enable_thinking\":false},\
\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}" 2>/dev/null)"
  printf '%s' "$out" | grep -q '"choices"'
}

printf '%-9s %-12s %-9s %s\n' "parallel" "max ctx" "per-seat" "note"
printf -- '---------------------------------------------------------\n'

IFS=',' read -r -a levels <<< "$PARALLEL_LIST"
for p in "${levels[@]}"; do
  set_key parallel "$p"
  ctx="$START_CTX"
  found=""
  while [ "$ctx" -ge "$STEP" ]; do
    set_key c "$ctx"
    if try_load; then found="$ctx"; break; fi
    ctx=$((ctx - STEP))
  done
  if [ -n "$found" ]; then
    free="$(nvidia-smi --query-gpu=memory.free --format=csv,noheader,nounits | awk '{s+=$1} END{print s+0}')"
    printf '%-9s %-12s %-9s %s\n' "$p" "$found" "$((found / p))" "${free} MiB free"
  else
    printf '%-9s %-12s %-9s %s\n' "$p" "none" "-" "no context fits"
  fi
done
