#!/usr/bin/env bash
# growth-adapter-gsc.sh — fetch Google Search Console rows, emit normalized
# envelope. Fetch via GROWTH_FETCH_CMD. Fail-closed.
set -euo pipefail
cfg="${1:-}"; [ -n "$cfg" ] || { echo "usage: growth-adapter-gsc.sh <config.json>" >&2; exit 2; }
[ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
here="$(cd "$(dirname "$0")" && pwd)"
site="$(jq -r '.measurement.gsc.site // ""' "$cfg")"
tok_env="$(jq -r '.measurement.gsc.token_env // "GSC_TOKEN"' "$cfg")"
[ -n "$site" ] || exit 1
tok="$(eval "printf '%s' \"\${$tok_env:-}\"")"
[ -n "$tok" ] || exit 1
url="https://searchconsole.googleapis.com/webmasters/v3/sites/$site/searchAnalytics/query"
fetch="${GROWTH_FETCH_CMD:-curl -fsSL}"
raw="$($fetch "$url")" || { echo "gsc adapter: fetch failed" >&2; exit 1; }
tmp="$(mktemp)"; printf '%s' "$raw" > "$tmp"
bash "$here/growth-measure.sh" --normalize gsc "$tmp"
rm -f "$tmp"
