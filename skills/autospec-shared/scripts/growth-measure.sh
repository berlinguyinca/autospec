#!/usr/bin/env bash
# growth-measure.sh — measurement-adapter normalization seam for autospec-grow.
# v1 normalizes raw provider payloads into a common envelope; live API callers
# plug into --normalize downstream (Plan 4).
set -euo pipefail

now_epoch() { echo "${GROWTH_NOW_EPOCH:-$(date -u +%s)}"; }

normalize() {
  local provider="${1:?provider required}" raw="${2:?raw json required}"
  [ -f "$raw" ] || { echo "raw not found: $raw" >&2; exit 2; }
  local ts; ts="$(now_epoch)"
  case "$provider" in
    gsc)
      jq --argjson ts "$ts" '{
        provider: "gsc",
        metrics: {
          clicks_total: ([.rows[].clicks] | add // 0),
          impressions_total: ([.rows[].impressions] | add // 0),
          queries: [.rows[] | {query: .keys[0], clicks, impressions, position}]
        },
        ts: $ts }' "$raw"
      ;;
    github)
      jq --argjson ts "$ts" '{
        provider: "github",
        metrics: { stars: (.stargazers_count // 0), forks: (.forks_count // 0) },
        ts: $ts }' "$raw"
      ;;
    *)
      echo "unknown provider: $provider" >&2; exit 1 ;;
  esac
}

cmd="${1:-}"; shift || true
case "$cmd" in
  --normalize) normalize "$@" ;;
  *) echo "usage: growth-measure.sh --normalize <provider> <raw.json>" >&2; exit 2 ;;
esac
