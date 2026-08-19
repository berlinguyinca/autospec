#!/usr/bin/env bash
# Add a GGUF model to the router: download it, write a preset, verify it serves.
#
#   add-gguf-model.sh --repo REPO --file NAME.gguf [--mmproj NAME.gguf]
#                     [--id MODEL_ID] [--context N] [--no-restart]
#
# Use for any GGUF of the same base architecture -- including abliterated /
# "uncensored" community builds, which are ordinary model choices here. They are
# third-party edits of the base weights, so this pins the repo revision it
# fetched and re-verifies the model actually answers. Capability is NOT assumed
# to survive abliteration; measure it.
set -euo pipefail

CONF_DIR="${QWEN38_CONF_DIR:-/opt/qwen-vllm/etc}"
# shellcheck source=../config/common.conf
. "${CONF_DIR}/common.conf"
PRESETS="${CONF_DIR}/router-presets.ini"
GGUF_DIR="/var/lib/qwen-gguf/models"
SVC_USER="qwen-vllm"

REPO=""; FILE=""; MMPROJ=""; MODEL_ID=""; CONTEXT=""; RESTART=1
while [ $# -gt 0 ]; do
  case "$1" in
    --repo)       REPO="$2"; shift 2 ;;
    --file)       FILE="$2"; shift 2 ;;
    --mmproj)     MMPROJ="$2"; shift 2 ;;
    --id)         MODEL_ID="$2"; shift 2 ;;
    --context)    CONTEXT="$2"; shift 2 ;;
    --no-restart) RESTART=0; shift ;;
    *) echo "unknown option: $1" >&2; exit 64 ;;
  esac
done
[ -n "$REPO" ] && [ -n "$FILE" ] || { echo "usage: $(basename "$0") --repo REPO --file NAME.gguf" >&2; exit 64; }
[ -r "$PRESETS" ] || { echo "router presets not found: ${PRESETS}" >&2; exit 69; }

# Default id from the filename, lowercased and slugified, so it is a valid and
# predictable OpenAI "model" value.
if [ -z "$MODEL_ID" ]; then
  MODEL_ID="$(printf '%s' "${FILE%.gguf}" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9.-]\+/-/g; s/^-//; s/-$//')"
fi
if grep -q "^\[${MODEL_ID}\]" "$PRESETS"; then
  echo "preset [${MODEL_ID}] already present in ${PRESETS}; nothing to do"
  exit 0
fi

HF="${QWEN38_VENV}/bin/hf"; [ -x "$HF" ] || HF="$(command -v hf || true)"
[ -n "$HF" ] || { echo "huggingface CLI not found" >&2; exit 69; }

fetch() {  # $1 = filename in the repo
  local f="$1"
  if [ -s "${GGUF_DIR}/${f}" ]; then echo "  have ${f}"; return 0; fi
  echo "  fetching ${f} from ${REPO}"
  sudo -u "${SVC_USER}" env HF_HOME=/var/lib/qwen-gguf/hf \
    "$HF" download "$REPO" "$f" >/dev/null
  local src; src="$(sudo find /var/lib/qwen-gguf/hf -name "$f" | head -1)"
  [ -n "$src" ] || { echo "download produced no ${f}" >&2; exit 1; }
  sudo cp -f "$(sudo readlink -f "$src")" "${GGUF_DIR}/${f}"
  sudo chown "${SVC_USER}:${SVC_USER}" "${GGUF_DIR}/${f}"
}

echo "adding ${MODEL_ID}"
fetch "$FILE"
[ -n "$MMPROJ" ] && fetch "$MMPROJ"

# A weights file this small is a failed or partial download, not a small quant.
sz="$(stat -Lc %s "${GGUF_DIR}/${FILE}")"
[ "$sz" -gt 3000000000 ] || { echo "suspiciously small (${sz} bytes): ${FILE}" >&2; exit 1; }

REV="$(curl -fsSL "https://huggingface.co/api/models/${REPO}" \
        | python3 -c 'import sys,json;print(json.load(sys.stdin).get("sha","")[:12])' 2>/dev/null || echo unknown)"

# Ask the selector what context this size can hold, rather than guessing.
if [ -z "$CONTEXT" ]; then
  # Consume --json, never the human table: scraping columns broke silently once
  # the table gained a column, and the fallback quietly under-served context.
  CONTEXT="$("${QWEN38_VENV}/bin/python" "$(dirname "$0")/select-quant.py" \
      --repo "$REPO" --json ${MMPROJ:+--vision} 2>/dev/null \
    | python3 -c 'import sys,json
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
f=sys.argv[1]
for c in d.get("candidates",[]):
    if c["file"]==f: print(c["max_ctx"]); break' "$FILE")"
  # Keep a margin under the arithmetic bound; it is an upper bound, not a limit.
  if [ -n "$CONTEXT" ] && [ "$CONTEXT" -gt 0 ]; then
    CONTEXT=$(( CONTEXT * 80 / 100 / 1568 * 1568 ))
  else
    CONTEXT=65536
  fi
fi

# Built line-by-line, NOT with $(... ) inside a heredoc: command substitution
# strips trailing newlines, which silently produced
#   image-min-tokens = 1024c = 65536
# -- a line llama.cpp ignores, so the model loaded with a default context and
# every long request 400'd.
{
  echo
  echo "; ${REPO} @ ${REV} -- added $(date -u +%Y-%m-%d) by add-gguf-model.sh"
  echo "; Context is an ARITHMETIC bound with a 20% margin, not a measured ceiling."
  echo "; Confirm with: long-prompt-probe.py <base> ${MODEL_ID} ${CONTEXT}"
  echo "[${MODEL_ID}]"
  echo "model = ${GGUF_DIR}/${FILE}"
  if [ -n "$MMPROJ" ]; then
    echo "mmproj = ${GGUF_DIR}/${MMPROJ}"
    echo "image-min-tokens = 1024"
  fi
  echo "c = ${CONTEXT}"
} | sudo tee -a "$PRESETS" >/dev/null

# A malformed preset is worse than a missing one: llama.cpp may still load the
# model while ignoring the context. Assert the key landed on its own line.
grep -qE "^c = ${CONTEXT}\$" "$PRESETS" \
  || { echo "preset for ${MODEL_ID} is malformed in ${PRESETS}" >&2; exit 1; }

echo "  preset [${MODEL_ID}] c=${CONTEXT} -> ${PRESETS}"

if [ "$RESTART" -eq 1 ]; then
  echo "restarting the router"
  qwen38ctl restart >/dev/null 2>&1 || qwen38ctl start router >/dev/null
  port="$(grep -m1 '^QWEN38_PORT=' "${CONF_DIR}/profiles.d/router.conf" | cut -d'"' -f2)"
  BASE="http://${QWEN38_HOST}:${port}"
  curl -sf --max-time 30 "${BASE}/v1/models" | grep -q "\"${MODEL_ID}\"" \
    || { echo "router does not advertise ${MODEL_ID}" >&2; exit 1; }
  echo "  advertised by the router"
  out="$(curl -sf --max-time 900 "${BASE}/v1/chat/completions" -H 'Content-Type: application/json' \
    -d "{\"model\":\"${MODEL_ID}\",\"temperature\":0,\"max_tokens\":64,
         \"messages\":[{\"role\":\"user\",\"content\":\"Reply with exactly: ADDED_OK\"}]}" \
    | python3 -c 'import sys,json
try: print(json.load(sys.stdin)["choices"][0]["message"]["content"])
except Exception: print("")')"
  printf '%s' "$out" | grep -q ADDED_OK \
    || { echo "model loaded but did not answer as asked: ${out:0:160}" >&2; exit 1; }
  echo "  PASS  served a completion"
fi

echo
echo "OK — select \"${MODEL_ID}\" in any client on the router endpoint."
echo "     Then re-run configure-opencode.py to expose it there."
