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
#     outcome     string  pending | merged_clean | qa_failed | reverted | stalled | abandoned | refuted
#                 `refuted` records a verify-stage refutation (Issue D): the
#                 proposal was dropped before becoming an issue, so it carries
#                 issue=0 and feeds the per-source refutation rate (down-weight)
#                 WITHOUT counting toward `filed`.
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
#   explore-ledger.sh --rebuild [--repo <owner/name>] [--sandbox <branch>] [--stalled-days N]
#   explore-ledger.sh -h | --help
#
# --rebuild reconstructs the ledger from GitHub history. It finds explore-filed
# issues carrying the canonical marker line in their body:
#
#   <!-- explore-ledger source=<src> complexity=<small|medium|large> confidence=<0.00-1.00> round=<N> -->
#
# parses source/complexity/confidence/round from the marker, derives each
# proposal's outcome + linked PR from issue + PR state, and writes a fresh,
# schema-valid ledger. It NEVER clobbers an existing live ledger: if the ledger
# path already exists, the rebuilt ledger is written to <ledger>.rebuilt for the
# operator to diff/promote. Requires gh AND jq (fail-closed, exit 2).
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

ALLOWED_OUTCOMES="pending merged_clean qa_failed reverted stalled abandoned refuted"
ALLOWED_COMPLEXITY="small medium large"

_usage() {
  sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
}

_die() { printf 'explore-ledger: %s\n' "$1" >&2; exit "${2:-1}"; }

# jq is mandatory; fail closed.
_require_jq() {
  command -v jq >/dev/null 2>&1 || _die "jq not found (required for ledger integrity)" 2
}

# gh is mandatory for --rebuild; fail closed.
_require_gh() {
  command -v gh >/dev/null 2>&1 || _die "gh not found (required for --rebuild)" 2
}

# _parse_marker <issue-body> — print "source<TAB>complexity<TAB>confidence<TAB>round"
# if the body contains exactly one parseable explore-ledger marker, else fail (1).
# Marker grammar (canonical):
#   <!-- explore-ledger source=<src> complexity=<small|medium|large> confidence=<0.00-1.00> round=<N> -->
# Field order inside the marker is fixed (source, complexity, confidence, round).
_parse_marker() {
  printf '%s' "$1" | sed -n -E \
    's/.*<!-- *explore-ledger +source=([^ ]+) +complexity=(small|medium|large) +confidence=([0-9]+(\.[0-9]+)?) +round=([0-9]+) *-->.*/\1	\2	\3	\5/p' \
    | head -n 1
}

# _normalize_title <title> — mirror explore-research-cycle.sh normalize_title:
# lowercase, strip a leading conventional-commit prefix, collapse non-alnum runs
# to single spaces, trim, cap at 120 chars.
_normalize_title() {
  printf '%s' "$1" \
    | tr '[:upper:]' '[:lower:]' \
    | sed -E 's/^(feat|fix|chore|docs|test|refactor|perf|track|ci)(\([^)]*\))?!?: *//' \
    | sed -E 's/[^a-z0-9]+/ /g' \
    | sed -E 's/^ +//; s/ +$//' \
    | cut -c1-120
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
RB_REPO=""
RB_SANDBOX=""
RB_STALLED_DAYS=7

while [ $# -gt 0 ]; do
  case "$1" in
    --ledger) LEDGER="$2"; shift 2 ;;
    --json) JSON=1; shift ;;
    --source) FILTER_SOURCE="$2"; shift 2 ;;
    --repo) RB_REPO="$2"; shift 2 ;;
    --sandbox) RB_SANDBOX="$2"; shift 2 ;;
    --stalled-days) RB_STALLED_DAYS="$2"; shift 2 ;;
    --rebuild) CMD="rebuild"; shift ;;
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
        | .[$s] //= {filed:0, merged_clean:0, failed:0, pending:0, refuted:0}
        | if   $r.outcome == "refuted" then .[$s].refuted += 1
          else .[$s].filed += 1
               | if   $r.outcome == "merged_clean" then .[$s].merged_clean += 1
                 elif ($r.outcome == "qa_failed" or $r.outcome == "reverted" or $r.outcome == "abandoned") then .[$s].failed += 1
                 elif $r.outcome == "pending" then .[$s].pending += 1
                 else . end
          end)
    ')"
    if [ "$JSON" -eq 1 ]; then
      printf '%s\n' "$stats_json"
    else
      printf '%-22s %6s %12s %6s %7s %7s\n' "source" "filed" "merged_clean" "failed" "pending" "refuted"
      printf '%s' "$stats_json" | jq -r 'to_entries[] | [.key, (.value.filed|tostring), (.value.merged_clean|tostring), (.value.failed|tostring), (.value.pending|tostring), ((.value.refuted // 0)|tostring)] | @tsv' \
        | while IFS=$'\t' read -r s f m fa p rf; do
            printf '%-22s %6s %12s %6s %7s %7s\n' "$s" "$f" "$m" "$fa" "$p" "$rf"
          done
    fi
    exit 0
    ;;

  rebuild)
    _require_gh
    repo_args=""
    [ -n "$RB_REPO" ] && repo_args="--repo $RB_REPO"

    # 1) Find candidate issues carrying the explore-ledger marker.
    issues_json="$(gh issue list $repo_args --state all \
      --search "explore-ledger in:body" \
      --json number,title,body,state,closedAt --limit 500 2>/dev/null)" \
      || _die "gh issue list failed" 2
    [ -n "$issues_json" ] || issues_json="[]"

    now_epoch="$(date -u +%s)"
    ts="$(_now_ts)"

    n_issues=0; n_parsed=0; n_skipped=0
    c_merged=0; c_pending=0; c_stalled=0; c_abandoned=0

    # Collect rebuilt records into a temp file (one JSON object per line).
    rebuilt_tmp="$(mktemp)"

    # Iterate issue numbers; pull each field via jq to stay newline-safe.
    issue_nums="$(printf '%s' "$issues_json" | jq -r '.[].number')"
    for num in $issue_nums; do
      n_issues=$((n_issues + 1))
      body="$(printf '%s' "$issues_json" | jq -r --argjson n "$num" '.[] | select(.number==$n) | .body // ""')"
      title="$(printf '%s' "$issues_json" | jq -r --argjson n "$num" '.[] | select(.number==$n) | .title // ""')"
      istate="$(printf '%s' "$issues_json" | jq -r --argjson n "$num" '.[] | select(.number==$n) | .state // ""')"

      marker="$(_parse_marker "$body")"
      if [ -z "$marker" ]; then
        n_skipped=$((n_skipped + 1))
        printf 'explore-ledger rebuild: issue #%s has no parseable marker (skipped)\n' "$num" >&2
        continue
      fi
      n_parsed=$((n_parsed + 1))

      src="$(printf '%s' "$marker" | cut -f1)"
      cx="$(printf '%s' "$marker" | cut -f2)"
      conf="$(printf '%s' "$marker" | cut -f3)"
      rnd="$(printf '%s' "$marker" | cut -f4)"
      norm="$(_normalize_title "$title")"

      # 2) Derive linked PR + per-issue timestamps.
      pr_json="$(gh pr list $repo_args --state all --search "#$num in:body" \
        --json number,state,mergedAt,baseRefName --limit 50 2>/dev/null)"
      [ -n "$pr_json" ] || pr_json="[]"
      iv_json="$(gh issue view $num $repo_args --json number,state,closedAt,createdAt,updatedAt 2>/dev/null)"
      [ -n "$iv_json" ] || iv_json="{}"

      # Merged PR (mergedAt non-null) takes precedence.
      merged_pr="$(printf '%s' "$pr_json" | jq -r '[.[] | select(.mergedAt != null)] | (.[0].number // empty)')"
      open_pr="$(printf '%s' "$pr_json" | jq -r '[.[] | select((.state|ascii_upcase)=="OPEN")] | (.[0].number // empty)')"

      created_at="$(printf '%s' "$iv_json" | jq -r '.createdAt // ""')"
      updated_at="$(printf '%s' "$iv_json" | jq -r '.updatedAt // ""')"
      closed_at="$(printf '%s' "$iv_json" | jq -r '.closedAt // ""')"
      iv_state="$(printf '%s' "$iv_json" | jq -r '.state // ""')"
      [ -n "$iv_state" ] && istate="$iv_state"
      istate_uc="$(printf '%s' "$istate" | tr '[:lower:]' '[:upper:]')"

      # 3) Outcome derivation.
      outcome="pending"; pr=0
      if [ -n "$merged_pr" ]; then
        outcome="merged_clean"; pr="$merged_pr"
      elif [ "$istate_uc" = "OPEN" ] && [ -n "$open_pr" ]; then
        outcome="pending"; pr="$open_pr"
      elif [ "$istate_uc" = "OPEN" ] && [ -z "$open_pr" ]; then
        # Stalled only if the issue has gone quiet: time since last update
        # (not creation) exceeds the threshold. An actively-draining issue
        # filed long ago but recently touched is NOT stalled.
        ref_ts="${updated_at:-$created_at}"
        age_days=0
        if [ -n "$ref_ts" ]; then
          ref_epoch="$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$ref_ts" +%s 2>/dev/null || date -u -d "$ref_ts" +%s 2>/dev/null || echo "")"
          if [ -n "$ref_epoch" ]; then
            age_days=$(( (now_epoch - ref_epoch) / 86400 ))
          fi
        fi
        if [ "$age_days" -gt "$RB_STALLED_DAYS" ]; then
          outcome="stalled"; pr=0
        else
          outcome="pending"; pr=0
        fi
      elif [ "$istate_uc" = "CLOSED" ]; then
        outcome="abandoned"; pr=0
      fi

      case "$outcome" in
        merged_clean) c_merged=$((c_merged + 1)) ;;
        pending)      c_pending=$((c_pending + 1)) ;;
        stalled)      c_stalled=$((c_stalled + 1)) ;;
        abandoned)    c_abandoned=$((c_abandoned + 1)) ;;
      esac

      # 4) Build a schema-valid record via jq (typed numbers, escaped strings).
      rec="$(jq -cn \
        --argjson round "$rnd" \
        --arg source "$src" \
        --arg title "$title" \
        --arg norm "$norm" \
        --arg complexity "$cx" \
        --argjson confidence "$conf" \
        --argjson issue "$num" \
        --argjson pr "$pr" \
        --arg outcome "$outcome" \
        --arg ts "$ts" \
        '{round:$round, source:$source, title:$title, norm_title:$norm, complexity:$complexity, confidence:$confidence, issue:$issue, pr:$pr, outcome:$outcome, reason:"rebuilt from gh history", ts:$ts}')" \
        || { rm -f "$rebuilt_tmp"; _die "could not build record for issue $num" 1; }
      _validate_object "$rec" || { rm -f "$rebuilt_tmp"; _die "rebuilt record for issue $num fails schema" 1; }
      printf '%s\n' "$rec" >> "$rebuilt_tmp"
    done

    # 5) Choose output path: never clobber a live ledger.
    if [ -f "$LEDGER" ]; then
      out="$LEDGER.rebuilt"
    else
      out="$LEDGER"
    fi
    mkdir -p "$(dirname "$out")"
    cp "$rebuilt_tmp" "$out"
    rm -f "$rebuilt_tmp"

    printf 'explore-ledger rebuild: issues=%s parsed=%s skipped_no_marker=%s merged_clean=%s pending=%s stalled=%s abandoned=%s -> %s\n' \
      "$n_issues" "$n_parsed" "$n_skipped" "$c_merged" "$c_pending" "$c_stalled" "$c_abandoned" "$out"
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
