#!/usr/bin/env bash
# Smoke tests — the gate setup-linux-qwen38.sh will not declare success without.
#
# Requires a running profile. Starts the default one if nothing is up.
set -uo pipefail

CONF_DIR="${QWEN38_CONF_DIR:-/opt/qwen-vllm/etc}"
# shellcheck source=../config/common.conf
. "${CONF_DIR}/common.conf"
BASE="http://${QWEN38_HOST}:${QWEN38_PORT}"

pass=0; fail=0
ok()   { printf '  PASS  %s\n' "$*"; pass=$((pass+1)); }
bad()  { printf '  FAIL  %s\n' "$*"; fail=$((fail+1)); }
check(){ if [ "$1" = 0 ]; then ok "$2"; else bad "$2${3:+ — $3}"; fi; }

if ! curl -sS --fail --max-time 5 "${BASE}/health" >/dev/null 2>&1; then
  echo "no healthy server on ${BASE}; starting default profile"
  qwen38ctl start interactive || { echo "could not start"; exit 1; }
fi

echo "== smoke =="

# 1 — health
curl -sS --fail --max-time 10 "${BASE}/health" >/dev/null 2>&1
check $? "/health responds"

# 2 — /v1/models advertises our alias
models="$(curl -sS --fail --max-time 10 "${BASE}/v1/models" 2>/dev/null)"
printf '%s' "$models" | grep -q "\"${QWEN38_SERVED_NAME}\""
check $? "/v1/models advertises ${QWEN38_SERVED_NAME}"

# 3 — max_model_len matches the profile that is actually running
served_len="$(printf '%s' "$models" | python3 -c 'import sys,json
d=json.load(sys.stdin); print(d["data"][0].get("max_model_len",""))' 2>/dev/null)"
[ -n "$served_len" ] && [ "$served_len" -gt 0 ]
check $? "/v1/models reports max_model_len (${served_len:-none})"

# NOTE: every generation test below passes enable_thinking=false and a generous
# max_tokens. Qwen3.8 is a reasoning model and thinks by DEFAULT -- ask it to
# "reply with exactly X" in 16 tokens and it spends all of them on "We need
# to...", never reaching X. That is the model behaving correctly; a test that
# omits this is testing its own prompt, not the server.

# 4 — simple completion
resp="$(curl -sS --fail --max-time 120 "${BASE}/v1/chat/completions" \
  -H 'Content-Type: application/json' -d "{
    \"model\": \"${QWEN38_SERVED_NAME}\", \"temperature\": 0, \"max_tokens\": 64,
    \"chat_template_kwargs\": {\"enable_thinking\": false},
    \"messages\": [{\"role\":\"user\",\"content\":\"Reply with exactly: SMOKE_OK\"}]
  }" 2>/dev/null)"
printf '%s' "$resp" | grep -q 'SMOKE_OK'
check $? "non-streaming completion returns the requested token" "$(printf '%s' "$resp" | head -c 200)"

# 5 — streaming
stream="$(curl -sS --fail --max-time 120 -N "${BASE}/v1/chat/completions" \
  -H 'Content-Type: application/json' -d "{
    \"model\": \"${QWEN38_SERVED_NAME}\", \"stream\": true, \"temperature\": 0, \"max_tokens\": 64,
    \"chat_template_kwargs\": {\"enable_thinking\": false},
    \"messages\": [{\"role\":\"user\",\"content\":\"Reply with exactly: STREAM_OK\"}]
  }" 2>/dev/null)"
# The deltas must be reassembled before matching. Grepping the raw SSE text for
# the expected string cannot work: the tokeniser splits it across chunks
# ("STREAM" in one frame, "_OK" in the next), so the literal never appears on
# any single line even when the response is perfectly correct.
assembled="$(printf '%s' "$stream" | python3 -c '
import json, sys
out = []
for line in sys.stdin:
    if not line.startswith("data: "):
        continue
    chunk = line[6:].strip()
    if chunk == "[DONE]":
        break
    try:
        choices = json.loads(chunk).get("choices") or []
    except json.JSONDecodeError:
        continue
    if choices:
        out.append(choices[0].get("delta", {}).get("content") or "")
print("".join(out))')"
printf '%s' "$stream" | grep -q 'data: ' && printf '%s' "$assembled" | grep -q 'STREAM_OK'
check $? "streaming completion emits SSE chunks and the requested token" "assembled: $(printf '%s' "$assembled" | head -c 120)"

# 6 — deterministic code generation, actually executed
code="$(curl -sS --fail --max-time 300 "${BASE}/v1/chat/completions" \
  -H 'Content-Type: application/json' -d "{
    \"model\": \"${QWEN38_SERVED_NAME}\", \"temperature\": 0, \"max_tokens\": 700,
    \"chat_template_kwargs\": {\"enable_thinking\": false},
    \"messages\": [{\"role\":\"user\",\"content\":\"Write a Python function merge_intervals(intervals). Return sorted merged intervals as lists, merge overlapping and touching intervals, accept unsorted input, return [] for empty input, and never mutate the input. Output only executable Python code, no markdown.\"}]
  }" 2>/dev/null | python3 -c 'import sys,json
try: print(json.load(sys.stdin)["choices"][0]["message"]["content"])
except Exception: pass')"

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
# Strip a markdown fence if the model added one anyway.
printf '%s' "$code" | sed -e '/^```/d' > "${tmp}/m.py"
cat > "${tmp}/t.py" <<'PY'
import copy, sys
sys.path.insert(0, sys.argv[1])
from m import merge_intervals
CASES = [
    ([], []),
    ([[1,3],[2,6],[8,10],[15,18]], [[1,6],[8,10],[15,18]]),
    ([[5,7],[1,2],[2,5]],          [[1,7]]),
    ([[4,4],[4,4]],                [[4,4]]),
    ([[-3,-1],[-2,2],[10,11]],     [[-3,2],[10,11]]),
]
good = 0
for src, want in CASES:
    keep = copy.deepcopy(src)
    try:
        got = merge_intervals(src)
    except Exception:
        continue
    if got == want and src == keep:
        good += 1
print(good)
PY
got="$("${QWEN38_VENV}/bin/python" "${tmp}/t.py" "$tmp" 2>/dev/null || echo 0)"
[ "${got:-0}" -ge 4 ]
check $? "generated merge_intervals passes >=4/5 cases without mutating input (got ${got:-0}/5)"

echo "== smoke: ${pass} passed, ${fail} failed =="
[ "$fail" -eq 0 ]
