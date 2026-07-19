#!/usr/bin/env bash
# growth-candidate-rank.sh — attach a deterministic rank_score and sort desc.
set -euo pipefail

CANDS="${1:?usage: growth-candidate-rank.sh <candidates.jsonl>}"
if [ ! -f "$CANDS" ]; then echo "candidates not found: $CANDS" >&2; exit 2; fi

HERE="$(cd "$(dirname "$0")" && pwd)"
WEIGHTS="$("$HERE/growth-source-weights.sh" 2>/dev/null || echo '{}')"

jq -sc --argjson w "$WEIGHTS" '
  def effort_factor: {"small":1.0,"medium":0.7,"large":0.4}[.effort] // 0.4;
  map(
    . as $c
    | ($w[$c.lens] // 0.5) as $sw
    | (((($c.roi/5)*0.5) + (($c.severity/5)*0.5)) * $c.confidence * ($c|effort_factor) * $sw) as $score
    | $c + {rank_score: (($score*100000|round)/100000)}
  )
  | sort_by([-.rank_score, -.severity, -.roi])
  | .[]
' "$CANDS"
