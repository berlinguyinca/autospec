#!/usr/bin/env bash
# growth-candidate-verify.sh — record an LLM adversarial-verify verdict.
# real:true  -> emit candidate; real:false or unparseable -> refute (ledger).
set -euo pipefail

CAND="${1:?usage: growth-candidate-verify.sh <candidate.json> <verdict.json>}"
VERDICT="${2:?usage: growth-candidate-verify.sh <candidate.json> <verdict.json>}"
if [ ! -f "$CAND" ]; then echo "candidate not found: $CAND" >&2; exit 2; fi

HERE="$(cd "$(dirname "$0")" && pwd)"
LEDGER_SH="$HERE/growth-ledger.sh"

refute() {
  local reason="$1"
  local lens title norm channel kind
  lens="$(jq -r '.lens // "unknown"' "$CAND")"
  title="$(jq -r '.title // ""' "$CAND")"
  norm="$(jq -r '.norm_title // ""' "$CAND")"
  channel="$(jq -r '.channel // ""' "$CAND")"
  kind="$(jq -r '.kind // "outbound"' "$CAND")"
  local line
  line="$(jq -n --arg s "$lens" --arg t "$title" --arg n "$norm" \
             --arg c "$channel" --arg k "$kind" --arg r "$reason" \
    '{round:1,source:$s,title:$t,norm_title:$n,channel:$c,kind:$k,issue:0,outcome:"refuted",reason:$r,ts:"1970-01-01T00:00:00Z"}')"
  "$LEDGER_SH" --append "$line"
}

# Fail-closed: unparseable verdict or non-boolean .real -> refute.
if [ ! -f "$VERDICT" ] || ! jq -e 'has("real") and (.real|type=="boolean")' "$VERDICT" >/dev/null 2>&1; then
  refute "unparseable verdict, refused"
  exit 0
fi

if jq -e '.real == true' "$VERDICT" >/dev/null; then
  jq -c . "$CAND"
else
  reason="$(jq -r '.reason // "refuted"' "$VERDICT")"
  refute "$reason"
fi
exit 0
