#!/usr/bin/env bash
# trend-ledger.sh — append-only JSONL ledger for autospec discovery trend signals.
# Mirrors growth-ledger.sh's append/latest/stats/validate pattern. Every appended
# record MUST pass skills/autospec-shared/scripts/validate-trend-signal.sh
# (fail-closed) before being written. Readers take the latest line per norm_key
# (exact-string match; never a regex compare).
set -euo pipefail

LEDGER="${AUTOSPEC_TREND_LEDGER:-.autospec/trends/ledger.jsonl}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VALIDATOR="$SCRIPT_DIR/validate-trend-signal.sh"

ensure_dir() { mkdir -p "$(dirname "$LEDGER")"; }

do_append() {
  local obj="${1:?json required}"
  echo "$obj" | jq -e . >/dev/null 2>&1 || { echo "not valid JSON" >&2; exit 2; }

  local tmp
  tmp="$(mktemp)"
  echo "$obj" | jq -c . > "$tmp"
  if ! "$VALIDATOR" "$tmp" >&2; then
    echo "trend signal rejected by validate-trend-signal.sh" >&2
    rm -f "$tmp"
    exit 1
  fi

  ensure_dir
  jq -c . "$tmp" >> "$LEDGER"
  rm -f "$tmp"
}

# latest line per norm_key (exact-string group-by; never a regex compare)
latest() {
  [ -f "$LEDGER" ] || { echo '[]'; return 0; }
  jq -s 'group_by(.norm_key) | map(.[-1])' "$LEDGER"
}

do_bump_recurrence() {
  local key="${1:?norm_key required}"
  [ -f "$LEDGER" ] || { echo "no ledger" >&2; exit 1; }
  local prev
  prev="$(jq -s --arg k "$key" 'map(select(.norm_key == $k)) | last' "$LEDGER")"
  if [ "$prev" = "null" ]; then echo "norm_key not found: $key" >&2; exit 1; fi
  local now
  now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "$prev" | jq -c --arg ts "$now" '.recurrence = ((.recurrence // 0) + 1) | .ts = $ts' >> "$LEDGER"
}

do_show() {
  local src="" json=0
  while [ $# -gt 0 ]; do case "$1" in
    --source) src="$2"; shift 2;; --json) json=1; shift;; *) shift;; esac; done
  local out; out="$(latest)"
  if [ -n "$src" ]; then out="$(echo "$out" | jq --arg s "$src" 'map(select(.source==$s))')"; fi
  if [ "$json" -eq 1 ]; then
    echo "$out"
  else
    echo "$out" | jq -r '.[] | "\(.source)\t\(.norm_key)\t\(.recurrence)\t\(.summary)"'
  fi
}

do_stats() {
  local rows; rows="$(latest)"
  if [ -z "$rows" ] || [ "$rows" = "null" ] || [ "$rows" = "[]" ]; then
    echo '{}'
    return 0
  fi
  echo "$rows" | jq 'group_by(.source) | map({key: .[0].source, value: length}) | from_entries'
}

do_validate() {
  local f="${1:-$LEDGER}"
  [ -f "$f" ] || { echo "no ledger: $f" >&2; exit 0; }
  local lineno=0 bad=0
  while IFS= read -r line; do
    lineno=$((lineno + 1))
    [ -n "$line" ] || continue
    local tmp
    tmp="$(mktemp)"
    printf '%s' "$line" > "$tmp"
    if ! "$VALIDATOR" "$tmp" >/dev/null 2>&1; then
      echo "invalid ledger line $lineno: $line" >&2
      bad=1
    fi
    rm -f "$tmp"
  done < "$f"
  if [ "$bad" -ne 0 ]; then exit 1; fi
  exit 0
}

cmd="${1:-}"; shift || true
case "$cmd" in
  --append)          do_append "$@" ;;
  --bump-recurrence)  do_bump_recurrence "$@" ;;
  --show)            do_show "$@" ;;
  --stats)           shift || true; do_stats ;;
  --validate)        do_validate "$@" ;;
  *) echo "usage: trend-ledger.sh --append|--bump-recurrence|--show|--stats|--validate" >&2; exit 2 ;;
esac
