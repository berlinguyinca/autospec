#!/usr/bin/env bash
# growth-adapter-rank.sh — fetch SERP rank/backlink data, emit normalized
# envelope. Fetch via GROWTH_FETCH_CMD. Fail-closed.
set -euo pipefail
cfg="${1:-}"; [ -n "$cfg" ] || { echo "usage: growth-adapter-rank.sh <config.json>" >&2; exit 2; }
[ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
here="$(cd "$(dirname "$0")" && pwd)"
endpoint="$(jq -r '.measurement.rank.endpoint // ""' "$cfg")"
tok_env="$(jq -r '.measurement.rank.token_env // "RANK_TOKEN"' "$cfg")"
[ -n "$endpoint" ] || { echo "rank adapter: .measurement.rank.endpoint missing" >&2; exit 1; }
tok="$(eval "printf '%s' \"\${$tok_env:-}\"")"
[ -n "$tok" ] || { echo "rank adapter: \$$tok_env unset (fail-closed)" >&2; exit 1; }
fetch="${GROWTH_FETCH_CMD:-curl -fsSL}"
raw="$($fetch "$endpoint")" || { echo "rank adapter: fetch failed" >&2; exit 1; }
tmp="$(mktemp)"; printf '%s' "$raw" > "$tmp"
bash "$here/growth-measure.sh" --normalize rank "$tmp"
rm -f "$tmp"
