#!/usr/bin/env bash
# growth-adapter-analytics.sh — fetch analytics visitors/pageviews (Plausible or
# GA4), emit normalized envelope. Fetch via GROWTH_FETCH_CMD. Fail-closed.
set -euo pipefail
cfg="${1:-}"; [ -n "$cfg" ] || { echo "usage: growth-adapter-analytics.sh <config.json>" >&2; exit 2; }
[ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
here="$(cd "$(dirname "$0")" && pwd)"
provider="$(jq -r '.measurement.analytics.provider // "plausible"' "$cfg")"
site="$(jq -r '.measurement.analytics.site // ""' "$cfg")"
tok_env="$(jq -r '.measurement.analytics.token_env // "PLAUSIBLE_API_TOKEN"' "$cfg")"
[ -n "$site" ] || exit 1
tok="$(eval "printf '%s' \"\${$tok_env:-}\"")"
[ -n "$tok" ] || exit 1
case "$provider" in
  plausible) url="https://plausible.io/api/v1/stats/aggregate?site_id=$site&metrics=visitors,pageviews" ;;
  ga4)       url="https://analyticsdata.googleapis.com/v1beta/properties/$site:runReport" ;;
  *) echo "analytics adapter: unknown provider $provider" >&2; exit 1 ;;
esac
fetch="${GROWTH_FETCH_CMD:-curl -fsSL}"
raw="$($fetch "$url")" || { echo "analytics adapter: fetch failed" >&2; exit 1; }
tmp="$(mktemp)"; printf '%s' "$raw" > "$tmp"
bash "$here/growth-measure.sh" --normalize analytics "$tmp"
rm -f "$tmp"
