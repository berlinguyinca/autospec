#!/usr/bin/env bash
# growth-ledger.sh — append-only JSONL outcome ledger for autospec-grow.
# Mirrors explore-ledger.sh: append-only, readers take latest line per issue.
# Outbound lines SHOULD also carry an optional "platform" string (e.g. "reddit"),
# consumed by growth-ethics-precheck.sh's cadence gate; --append stores extra
# keys verbatim, so this is a convention, not a REQUIRED key (artifact lines
# have no platform).
set -euo pipefail

LEDGER="${GROWTH_LEDGER:-.autospec/growth/ledger.jsonl}"
REQUIRED='["round","source","title","norm_title","channel","kind","issue","outcome","reason","ts"]'

ensure_dir() { mkdir -p "$(dirname "$LEDGER")"; }

do_append() {
  local obj="${1:?json required}"
  echo "$obj" | jq -e . >/dev/null || { echo "not valid JSON" >&2; exit 2; }
  local missing
  missing="$(echo "$obj" | jq -r --argjson req "$REQUIRED" '$req - (keys) | .[]' 2>/dev/null || true)"
  if [ -n "$missing" ]; then echo "missing keys: $missing" >&2; exit 1; fi
  ensure_dir
  echo "$(echo "$obj" | jq -c .)" >> "$LEDGER"
}

# latest line per issue (issue 0 lines are kept individually — they are refutations)
latest() {
  [ -f "$LEDGER" ] || return 0
  jq -s '
    (map(select(.issue != 0)) | group_by(.issue) | map(.[-1]))
    + (map(select(.issue == 0)))' "$LEDGER"
}

do_update_outcome() {
  local issue="${1:?}" outcome="${2:?}" reason="${3:-}"
  [ -f "$LEDGER" ] || { echo "no ledger" >&2; exit 1; }
  local prev
  prev="$(jq -s --argjson i "$issue" 'map(select(.issue==$i)) | last' "$LEDGER")"
  if [ "$prev" = "null" ]; then echo "issue $issue not found" >&2; exit 1; fi
  echo "$prev" | jq -c --arg o "$outcome" --arg r "$reason" '.outcome=$o | .reason=$r' >> "$LEDGER"
}

do_show() {
  local src="" json=0
  while [ $# -gt 0 ]; do case "$1" in
    --source) src="$2"; shift 2;; --json) json=1; shift;; *) shift;; esac; done
  local out; out="$(latest)"
  if [ -n "$src" ]; then out="$(echo "$out" | jq --arg s "$src" 'map(select(.source==$s))')"; fi
  if [ "$json" -eq 1 ]; then echo "$out"; else echo "$out" | jq -r '.[] | "\(.issue)\t\(.source)\t\(.outcome)\t\(.title)"'; fi
}

do_stats() {
  local rows; rows="$(latest)"
  if [ -z "$rows" ] || [ "$rows" = "null" ] || [ "$rows" = "[]" ]; then
    echo '{}'
    return 0
  fi
  echo "$rows" | jq '
    group_by(.source) | map({
      key: .[0].source,
      value: {
        filed:        (map(select(.issue != 0)) | length),
        merged_clean: (map(select(.issue != 0 and .outcome=="merged_clean")) | length),
        published:    (map(select(.issue != 0 and .outcome=="published")) | length),
        refuted:      (map(select(.outcome=="refuted")) | length)
      }
    }) | from_entries'
}

do_validate() {
  local f="${1:-$LEDGER}"
  [ -f "$f" ] || { echo "no ledger: $f" >&2; exit 0; }
  local bad
  bad="$(jq -c --argjson req "$REQUIRED" 'select(($req - (keys)) | length > 0)' "$f" 2>/dev/null || true)"
  if [ -n "$bad" ]; then echo "invalid ledger lines:" >&2; echo "$bad" >&2; exit 1; fi
  exit 0
}

cmd="${1:-}"; shift || true
case "$cmd" in
  --append)         do_append "$@" ;;
  --update-outcome) do_update_outcome "$@" ;;
  --show)           do_show "$@" ;;
  --stats)          shift || true; do_stats ;;
  --validate)       do_validate "$@" ;;
  *) echo "usage: growth-ledger.sh --append|--update-outcome|--show|--stats|--validate" >&2; exit 2 ;;
esac
