#!/usr/bin/env bash
# explore-ledger.sh — append-only JSONL outcome ledger for autospec-explore.
#
# The agentic-memory data engine: records what the explore loop PROPOSED and how
# each proposal turned out, so downstream tooling can dynamically re-weight the 6
# researcher sources and surface learnings. One compact JSON object per line.
#
# Record schema (all keys required for --append):
#   {round, source, title, norm_title, complexity, confidence, issue, pr, outcome, reason, ts}
#     round       integer >= 1
#     source      string  (researcher source name, e.g. spec-vs-code)
#     title       string  (original proposal title)
#     norm_title  string  (caller-supplied normalized title — stored AS GIVEN)
#     complexity  string  "small" | "medium" | "large"
#     confidence  number  0..1
#     issue       integer GitHub issue number, or 0 if not filed
#     pr          integer PR number, or 0/null if none
#     outcome     string  pending | merged_clean | qa_failed | reverted | stalled | abandoned
#     reason      string  (may be empty)
#     ts          ISO-8601 UTC string (caller-supplied for --append)
#
# Append-only audit trail: --update-outcome appends a NEW copy with an updated
# outcome/reason/ts rather than rewriting. Readers (--show / --stats) take the
# LATEST line per issue.
#
# Usage:
#   explore-ledger.sh --append '<json-object>'
#   explore-ledger.sh --update-outcome <issue> <outcome> [reason]
#   explore-ledger.sh --stats [--json]
#   explore-ledger.sh --show [--source <name>] [--json]
#   explore-ledger.sh --validate [<file>]
#   explore-ledger.sh -h | --help
#
# Ledger location (precedence): --ledger <path>  >  $AUTOSPEC_EXPLORE_LEDGER
#   >  .autospec/explore-ledger.jsonl
#
# Exit codes:
#   0  ok / valid
#   1  invalid object/line, or invalid outcome arg
#   2  jq missing (fail-closed), or --update-outcome issue not found
#
# Requires: bash 3.2+, jq, date (BSD/GNU). jq is MANDATORY — this is a
# data-integrity tool, so it fails closed (exit 2) when jq is absent.

set +e

ALLOWED_OUTCOMES="pending merged_clean qa_failed reverted stalled abandoned"
ALLOWED_COMPLEXITY="small medium large"

_usage() {
  sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
}

_die() { printf 'explore-ledger: %s\n' "$1" >&2; exit "${2:-1}"; }

# jq is mandatory; fail closed.
_require_jq() {
  command -v jq >/dev/null 2>&1 || _die "jq not found (required for ledger integrity)" 2
}

_now_ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }

# _outcome_ok <value> — 0 if value is in the allowed outcome set.
_outcome_ok() {
  local o
  for o in $ALLOWED_OUTCOMES; do [ "$1" = "$o" ] && return 0; done
  return 1
}

# _validate_object <json-string> — 0 if it satisfies the full record schema.
_validate_object() {
  printf '%s' "$1" | jq -e --arg outcomes "$ALLOWED_OUTCOMES" --arg cx "$ALLOWED_COMPLEXITY" '
    ($outcomes | split(" ")) as $oset
    | ($cx | split(" ")) as $cxset
    | (type == "object")
      and (.round | type == "number" and . >= 1)
      and (.source | type == "string")
      and (.title | type == "string")
      and (.norm_title | type == "string")
      and (.complexity | type == "string" and (. as $c | $cxset | index($c) != null))
      and (.confidence | type == "number" and . >= 0 and . <= 1)
      and (.issue | type == "number")
      and ((.pr == null) or (.pr | type == "number"))
      and (.outcome | type == "string" and (. as $o | $oset | index($o) != null))
      and (.reason | type == "string")
      and (.ts | type == "string" and (. != ""))
  ' >/dev/null 2>&1
}

# _latest_per_issue <file> — emit the latest line per issue as a JSON array.
# Lines whose issue is 0 (not filed) are kept individually (no dedup by issue).
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

# ── Argument parsing ─────────────────────────────────────────────────────────
LEDGER="${AUTOSPEC_EXPLORE_LEDGER:-.autospec/explore-ledger.jsonl}"
CMD=""
JSON=0
FILTER_SOURCE=""
APPEND_OBJ=""
UP_ISSUE=""
UP_OUTCOME=""
UP_REASON=""
VALIDATE_FILE=""

while [ $# -gt 0 ]; do
  case "$1" in
    --ledger) LEDGER="$2"; shift 2 ;;
    --json) JSON=1; shift ;;
    --source) FILTER_SOURCE="$2"; shift 2 ;;
    --append) CMD="append"; APPEND_OBJ="$2"; shift 2 ;;
    --update-outcome)
      CMD="update"; UP_ISSUE="$2"; UP_OUTCOME="$3"
      if [ $# -ge 4 ] && [ "${4#--}" = "$4" ]; then UP_REASON="$4"; shift 4; else UP_REASON=""; shift 3; fi
      ;;
    --stats) CMD="stats"; shift ;;
    --show) CMD="show"; shift ;;
    --validate)
      CMD="validate"
      if [ $# -ge 2 ] && [ "${2#--}" = "$2" ]; then VALIDATE_FILE="$2"; shift 2; else shift; fi
      ;;
    -h|--help) _usage; exit 0 ;;
    *) _die "unknown argument: $1" 1 ;;
  esac
done

[ -n "$CMD" ] || { _usage >&2; exit 1; }

_require_jq

case "$CMD" in
  append)
    [ -n "$APPEND_OBJ" ] || _die "--append requires a JSON object" 1
    _validate_object "$APPEND_OBJ" || _die "record fails schema validation" 1
    dir="$(dirname "$LEDGER")"
    mkdir -p "$dir"
    # Re-emit compact (single line) to guarantee one object per line.
    compact="$(printf '%s' "$APPEND_OBJ" | jq -c '.')" || _die "could not compact JSON" 1
    printf '%s\n' "$compact" >> "$LEDGER"
    exit 0
    ;;

  update)
    [ -n "$UP_ISSUE" ] && [ -n "$UP_OUTCOME" ] || _die "--update-outcome requires <issue> <outcome>" 1
    _outcome_ok "$UP_OUTCOME" || _die "invalid outcome: $UP_OUTCOME (allowed: $ALLOWED_OUTCOMES)" 1
    [ -f "$LEDGER" ] || _die "no record for issue $UP_ISSUE" 2
    # Find the most recent line matching the issue.
    latest="$(jq -c --argjson iss "$UP_ISSUE" 'select((.issue // 0) == $iss)' "$LEDGER" 2>/dev/null | tail -n 1)"
    [ -n "$latest" ] || _die "no record for issue $UP_ISSUE" 2
    ts="$(_now_ts)"
    updated="$(printf '%s' "$latest" | jq -c --arg o "$UP_OUTCOME" --arg r "$UP_REASON" --arg ts "$ts" \
      '.outcome = $o | .reason = $r | .ts = $ts')" || _die "could not build updated record" 1
    printf '%s\n' "$updated" >> "$LEDGER"
    exit 0
    ;;

  show)
    [ -f "$LEDGER" ] || { [ "$JSON" -eq 1 ] && echo "[]"; exit 0; }
    arr="$(_latest_per_issue "$LEDGER")"
    if [ -n "$FILTER_SOURCE" ]; then
      arr="$(printf '%s' "$arr" | jq -c --arg s "$FILTER_SOURCE" '[ .[] | select(.source == $s) ]')"
    fi
    if [ "$JSON" -eq 1 ]; then
      printf '%s\n' "$arr"
    else
      printf '%s' "$arr" | jq -r '.[] | "[\(.outcome)] #\(.issue) \(.source): \(.title) (r\(.round), \(.complexity), conf=\(.confidence))"'
    fi
    exit 0
    ;;

  stats)
    if [ ! -f "$LEDGER" ]; then
      [ "$JSON" -eq 1 ] && echo "{}" || printf 'no ledger at %s\n' "$LEDGER"
      exit 0
    fi
    arr="$(_latest_per_issue "$LEDGER")"
    stats_json="$(printf '%s' "$arr" | jq '
      reduce .[] as $r ({};
        ($r.source) as $s
        | .[$s] //= {filed:0, merged_clean:0, failed:0, pending:0}
        | .[$s].filed += 1
        | if   $r.outcome == "merged_clean" then .[$s].merged_clean += 1
          elif ($r.outcome == "qa_failed" or $r.outcome == "reverted" or $r.outcome == "abandoned") then .[$s].failed += 1
          elif $r.outcome == "pending" then .[$s].pending += 1
          else . end)
    ')"
    if [ "$JSON" -eq 1 ]; then
      printf '%s\n' "$stats_json"
    else
      printf '%-22s %6s %12s %6s %7s\n' "source" "filed" "merged_clean" "failed" "pending"
      printf '%s' "$stats_json" | jq -r 'to_entries[] | [.key, (.value.filed|tostring), (.value.merged_clean|tostring), (.value.failed|tostring), (.value.pending|tostring)] | @tsv' \
        | while IFS=$'\t' read -r s f m fa p; do
            printf '%-22s %6s %12s %6s %7s\n' "$s" "$f" "$m" "$fa" "$p"
          done
    fi
    exit 0
    ;;

  validate)
    target="${VALIDATE_FILE:-$LEDGER}"
    [ -f "$target" ] || _die "file not found: $target" 1
    rc=0
    n=0
    while IFS= read -r line || [ -n "$line" ]; do
      n=$((n + 1))
      [ -n "$line" ] || continue
      if ! _validate_object "$line"; then
        printf 'explore-ledger: invalid record at line %s\n' "$n" >&2
        rc=1
      fi
    done < "$target"
    exit "$rc"
    ;;
esac
