#!/usr/bin/env bash
# growth-source-weights.sh — Bayesian-smoothed per-source ranking weights,
# derived from the growth ledger. Mirrors explore-source-weights.sh.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
LEDGER_SH="$HERE/growth-ledger.sh"
ALPHA=5
PRIOR=0.5
SOURCES="technical-seo keyword-gap content-opportunity community directory backlink"

stats_json="$("$LEDGER_SH" --stats --json 2>/dev/null || echo '{}')"

emit="{}"
for s in $SOURCES; do
  w="$(echo "$stats_json" | jq -r --arg s "$s" --argjson a "$ALPHA" --argjson p "$PRIOR" '
    (.[$s] // {filed:0,merged_clean:0,published:0,refuted:0}) as $x
    | (($x.merged_clean + $x.published + ($a*$p)) / ($x.filed + $a)) as $base
    | (1 / (1 + ($x.refuted / ($x.filed + 1)))) as $dw
    | ($base * $dw)')"
  emit="$(echo "$emit" | jq --arg s "$s" --argjson w "$w" '.[$s] = ($w | (.*10000|round)/10000)')"
done

echo "$emit"
