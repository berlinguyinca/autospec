#!/usr/bin/env bash
# explore-learnings.sh — derive a human-readable learnings memo from the
# explore-ledger.jsonl outcome ledger.
#
# Closes the agentic-memory loop's reader side: the orchestrator writes the
# ledger (proposals + outcomes), this script distills it into a markdown memo
# operators (and the next research cycle) can skim:
#   - per-source clean-ship rate (sorted by clean rate desc)
#   - what ships cleanly here (recent merged_clean titles)
#   - recurring failure modes (recent qa_failed/reverted/abandoned titles+reason)
#
# It reads the ledger via explore-ledger.sh --stats --json (per-source counts)
# plus a direct scan of the latest record per issue for the title sections.
#
# Usage:
#   explore-learnings.sh [--ledger <path>] [--out <path>]
#   explore-learnings.sh -h | --help
#
# Ledger location (precedence): --ledger <path>  >  $AUTOSPEC_EXPLORE_LEDGER
#   >  .autospec/explore-ledger.jsonl
# Output  location (precedence): --out <path>    >  .autospec/explore-learnings.md
#
# Exit codes:
#   0  ok (memo written; empty/absent ledger still writes a valid memo)
#   1  bad argument
#   2  jq missing (fail-closed — this is a data tool)
#
# Requires: bash 3.2+, jq.

set +e

LEDGER="${AUTOSPEC_EXPLORE_LEDGER:-.autospec/explore-ledger.jsonl}"
OUT=".autospec/explore-learnings.md"

_usage() {
  sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
}

_die() { printf 'explore-learnings: %s\n' "$1" >&2; exit "${2:-1}"; }

# Resolve explore-ledger.sh defensively (for --stats --json). Order:
#   1. $AUTOSPEC_EXPLORE_LEDGER_BIN
#   2. $AUTOSPEC_SCRIPTS_DIR/explore-ledger.sh
#   3. sibling of this script
_resolve_ledger_bin() {
  if [ -n "${AUTOSPEC_EXPLORE_LEDGER_BIN:-}" ] && [ -f "${AUTOSPEC_EXPLORE_LEDGER_BIN}" ]; then
    printf '%s\n' "$AUTOSPEC_EXPLORE_LEDGER_BIN"; return 0
  fi
  if [ -n "${AUTOSPEC_SCRIPTS_DIR:-}" ] && [ -f "$AUTOSPEC_SCRIPTS_DIR/explore-ledger.sh" ]; then
    printf '%s\n' "$AUTOSPEC_SCRIPTS_DIR/explore-ledger.sh"; return 0
  fi
  local self_dir
  self_dir="$(cd "$(dirname "$0")" && pwd)"
  if [ -f "$self_dir/explore-ledger.sh" ]; then
    printf '%s\n' "$self_dir/explore-ledger.sh"; return 0
  fi
  printf '\n'
}

# ── Argument parsing ──────────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
  case "$1" in
    --ledger) LEDGER="$2"; shift 2 ;;
    --out)    OUT="$2"; shift 2 ;;
    -h|--help) _usage; exit 0 ;;
    *) _die "unknown argument: $1" 1 ;;
  esac
done

command -v jq >/dev/null 2>&1 || _die "jq not found (required for learnings)" 2

LEDGER_BIN="$(_resolve_ledger_bin)"
NOW_TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

mkdir -p "$(dirname "$OUT")" 2>/dev/null || true

# _latest_per_issue <file> — latest line per issue as a JSON array (mirrors
# explore-ledger.sh; issue==0 records kept individually).
_latest_per_issue() {
  jq -s '
    reduce .[] as $r ({order:[], by:{}};
      ($r.issue // 0) as $i
      | if $i == 0
        then .order += [("u" + (.order | length | tostring))]
             | .by[("u" + ((.order | length) - 1 | tostring))] = $r
        else (if (.by | has($i|tostring)) then . else .order += [($i|tostring)] end)
             | .by[($i|tostring)] = $r
        end)
    | [ .order[] as $k | .by[$k] ]
  ' "$1"
}

# ── Empty / absent ledger → valid memo noting no outcomes. ─────────────────────
if [ ! -f "$LEDGER" ] || [ ! -s "$LEDGER" ]; then
  {
    printf '# Explore Learnings (auto-derived from explore-ledger.jsonl)\n\n'
    printf '## Source performance (clean-ship rate)\n\n'
    printf '| Source | Filed | Merged clean | Failed | Clean rate |\n'
    printf '|--------|-------|--------------|--------|------------|\n'
    printf '\n_No outcomes recorded yet._\n\n'
    printf '## What ships cleanly here\n\n'
    printf '_No outcomes recorded yet._\n\n'
    printf '## Recurring failure modes\n\n'
    printf '_No outcomes recorded yet._\n\n'
    printf '## Generated: %s\n' "$NOW_TS"
  } > "$OUT"
  printf 'explore-learnings: wrote %s (no outcomes recorded yet)\n' "$OUT"
  exit 0
fi

# ── Source performance table via explore-ledger.sh --stats --json. ────────────
stats_json="{}"
if [ -n "$LEDGER_BIN" ]; then
  stats_json="$(bash "$LEDGER_BIN" --ledger "$LEDGER" --stats --json 2>/dev/null)"
  [ -n "$stats_json" ] || stats_json="{}"
fi

# Build table rows sorted by clean rate desc. clean_rate = merged_clean/filed.
table_rows="$(printf '%s' "$stats_json" | jq -r '
  to_entries
  | map({
      source: .key,
      filed: (.value.filed // 0),
      merged: (.value.merged_clean // 0),
      failed: (.value.failed // 0),
      rate: (if (.value.filed // 0) > 0 then ((.value.merged_clean // 0) / (.value.filed)) else 0 end)
    })
  | sort_by(-.rate, -.filed, .source)
  | .[]
  | "| \(.source) | \(.filed) | \(.merged) | \(.failed) | \((.rate * 100 | round))% |"
' 2>/dev/null)"

# ── Title sections from the latest record per issue. ──────────────────────────
arr="$(_latest_per_issue "$LEDGER" 2>/dev/null)"
[ -n "$arr" ] || arr="[]"

# Ships cleanly: up to 5 merged_clean titles, most recent first (by ts desc).
ships_clean="$(printf '%s' "$arr" | jq -r '
  [ .[] | select(.outcome == "merged_clean") ]
  | sort_by(.ts) | reverse | .[0:5] | .[]
  | "- \(.title)"
' 2>/dev/null)"

# Failure modes: up to 5 qa_failed/reverted/abandoned titles+reason, recent first.
fail_modes="$(printf '%s' "$arr" | jq -r '
  [ .[] | select(.outcome == "qa_failed" or .outcome == "reverted" or .outcome == "abandoned") ]
  | sort_by(.ts) | reverse | .[0:5] | .[]
  | "- \(.title) — \(.outcome)" + (if (.reason // "") != "" then ": \(.reason)" else "" end)
' 2>/dev/null)"

{
  printf '# Explore Learnings (auto-derived from explore-ledger.jsonl)\n\n'

  printf '## Source performance (clean-ship rate)\n\n'
  printf '| Source | Filed | Merged clean | Failed | Clean rate |\n'
  printf '|--------|-------|--------------|--------|------------|\n'
  if [ -n "$table_rows" ]; then
    printf '%s\n' "$table_rows"
  fi
  printf '\n'

  printf '## What ships cleanly here\n\n'
  if [ -n "$ships_clean" ]; then
    printf '%s\n' "$ships_clean"
  else
    printf '_No proposals have merged cleanly yet._\n'
  fi
  printf '\n'

  printf '## Recurring failure modes\n\n'
  if [ -n "$fail_modes" ]; then
    printf '%s\n' "$fail_modes"
  else
    printf '_No failure modes recorded yet._\n'
  fi
  printf '\n'

  printf '## Generated: %s\n' "$NOW_TS"
} > "$OUT"

printf 'explore-learnings: wrote %s\n' "$OUT"
exit 0
