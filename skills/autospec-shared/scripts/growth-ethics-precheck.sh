#!/usr/bin/env bash
# growth-ethics-precheck.sh — deterministic ethics pre-checks that gate a draft
# BEFORE any LLM ethics review. Fail-closed.
set -euo pipefail

WEEK_SECONDS=604800

usage() { echo "usage: growth-ethics-precheck.sh --disclosure <draft.md> | --cadence <cfg.json> <ledger.jsonl> <platform>" >&2; exit 2; }

now_epoch() { echo "${GROWTH_NOW_EPOCH:-$(date -u +%s)}"; }

check_disclosure() {
  local draft="${1:?draft required}"
  [ -f "$draft" ] || { echo "draft not found: $draft" >&2; exit 2; }
  local body; body="$(tr '[:upper:]' '[:lower:]' < "$draft")"
  local is_sponsored=0
  case "$body" in
    *affiliate*|*sponsored*|*"paid partnership"*) is_sponsored=1 ;;
  esac
  if [ "$is_sponsored" -eq 0 ]; then exit 0; fi
  # Marker must appear as a whole token, not as a substring of a larger word
  # (e.g. "#adventure" must NOT satisfy the "#ad" marker).
  if grep -Eiq -e '(^|[^[:alnum:]])#ad([^[:alnum:]]|$)' \
               -e '(^|[^[:alnum:]])#sponsored([^[:alnum:]]|$)' \
               -e 'disclosure:' "$draft"; then
    exit 0
  fi
  echo "sponsored/affiliate draft lacks an FTC disclosure marker (#ad, #sponsored, or 'Disclosure:')" >&2
  exit 1
}

check_cadence() {
  local cfg="${1:?}" ledger="${2:?}" platform="${3:?}"
  [ -f "$cfg" ] || { echo "config not found: $cfg" >&2; exit 2; }
  [ -f "$ledger" ] || { echo "ledger not found: $ledger" >&2; exit 2; }
  if ! jq . "$ledger" >/dev/null 2>&1; then
    echo "malformed ledger, refusing (fail-closed): $ledger" >&2
    exit 1
  fi
  local cap
  # The trailing `// 2` is the design default cadence cap when the config
  # omits both a community-specific cap and approval.cadence_caps.default_per_platform_per_week.
  cap="$(jq -r --arg p "$platform" \
    'first(.targets.communities[]? | select(.platform==$p) | .cadence_cap_per_week)
     // .approval.cadence_caps.default_per_platform_per_week // 2' "$cfg")"
  local now cutoff count
  now="$(now_epoch)"; cutoff=$(( now - WEEK_SECONDS ))
  if ! count="$(jq -s --arg p "$platform" --argjson cut "$cutoff" '
      map(select(.platform==$p and .outcome=="published"))
      | map(.ts | (if type=="number" then . else (fromdateiso8601? // error("unparseable ts")) end))
      | map(select(. >= $cut))
      | length' "$ledger")"; then
    echo "cadence: unparseable ledger timestamp, refusing (fail-closed): $ledger" >&2
    exit 1
  fi
  if [ "$count" -ge "$cap" ]; then
    echo "cadence cap reached for $platform: $count/$cap published in the last 7 days" >&2
    exit 1
  fi
  exit 0
}

cmd="${1:-}"; shift || true
case "$cmd" in
  --disclosure) check_disclosure "$@" ;;
  --cadence)    check_cadence "$@" ;;
  *) usage ;;
esac
