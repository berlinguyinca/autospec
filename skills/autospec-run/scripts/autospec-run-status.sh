#!/usr/bin/env bash
# autospec-run-status.sh — one-glance operator view of a running autospec-run.
#
# Aggregates the signals an operator otherwise hand-parses: per-issue heartbeats
# (step + age + branch + PR + host, with STALE detection), the queue counts, and
# the global stop-flag. Read-only; safe to run any time.
#
# Usage:
#   autospec-run-status.sh [--repo <owner/repo>] [--stale-secs N] [--json]
#
# Environment:
#   AUTOSPEC_HEARTBEAT_DIR       base heartbeat dir (default: ~/.autospec/process-heartbeats)
#   AUTOSPEC_STATE_DIR           state dir for the stop flag (default: ~/.autospec)
#   AUTOSPEC_REPO                repo override (owner/repo)
#   AUTOSPEC_WATCHDOG_STALE_SECS default stale threshold (default 1500 = 25 min)
#
# Exit codes:
#   0  rendered (whether or not work is active)
#   2  jq missing (cannot parse heartbeats)
#
# Requires: bash 3.2+, jq. gh is optional (queue counts degrade to n/a without it).

set +e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE_DIR="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}"
STALE_SECS="${AUTOSPEC_WATCHDOG_STALE_SECS:-1500}"
REPO_ARG="" JSON=0

while [ $# -gt 0 ]; do
  case "$1" in
    --repo) shift; REPO_ARG="${1:-}" ;;
    --stale-secs) shift; STALE_SECS="${1:-$STALE_SECS}" ;;
    --json) JSON=1 ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "autospec-run-status: unknown arg $1" >&2; exit 2 ;;
  esac
  shift
done

command -v jq >/dev/null 2>&1 || { echo "autospec-run-status: FATAL jq missing" >&2; exit 2; }

now="$(date -u +%s)"

# Resolve the repo slug for the heartbeat subdir (--repo > AUTOSPEC_REPO > gh).
repo="$REPO_ARG"
[ -n "$repo" ] || repo="${AUTOSPEC_REPO:-}"
if [ -z "$repo" ] && command -v gh >/dev/null 2>&1; then
  repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null)"
fi
HB_BASE="${AUTOSPEC_HEARTBEAT_DIR:-${AUTOSPEC_WATCHDOG_DIR:-$HOME/.autospec/process-heartbeats}}"

# Resolve heartbeat dirs canonical-first with legacy fallback (F4). This
# script runs under `set +e`, so we EXEC repo-slug.sh standalone rather than
# source it (sourcing would flip on `set -euo pipefail`). Resolution order:
# override → sibling (installed flat layout) → AUTOSPEC_SCRIPTS_DIR →
# repo-relative (dev/test checkout). Degraded fallback stays canonical
# (owner__name) so a reader never keys legacy against a canonical writer.
_rs_sh=""
for _c in \
  "${AUTOSPEC_REPO_SLUG_SH:-}" \
  "$SCRIPT_DIR/repo-slug.sh" \
  "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" \
  "$SCRIPT_DIR/../../../scripts/repo-slug.sh"; do
  if [ -n "$_c" ] && [ -f "$_c" ]; then _rs_sh="$_c"; break; fi
done
if [ -n "$repo" ]; then
  if [ -n "$_rs_sh" ]; then
    hb_dir="$(bash "$_rs_sh" --resolve-dir "$HB_BASE" "$repo" 2>/dev/null)"
  fi
  [ -n "${hb_dir:-}" ] || hb_dir="$HB_BASE/$(printf '%s' "$repo" | sed 's#/#__#')"
else
  hb_dir=""
fi

slug_dirs() {
  _base="$1"
  _repo="$2"
  _owner="${_repo%%/*}"
  _name="${_repo##*/}"
  _canonical="${_base}/${_owner}__${_name}"
  _legacy_under="${_base}/${_owner}_${_name}"
  _legacy_hyphen="${_base}/${_owner}-${_name}"
  printf '%s\n' "$_canonical"
  [ "$_legacy_under" = "$_canonical" ] || printf '%s\n' "$_legacy_under"
  [ "$_legacy_hyphen" = "$_canonical" ] || [ "$_legacy_hyphen" = "$_legacy_under" ] || printf '%s\n' "$_legacy_hyphen"
}

# Build a JSON array of per-issue rows by reading the heartbeat files directly
# (no dependency on heartbeat-read.sh being co-installed).
rows="[]"
if [ -n "$repo" ]; then
  hb_candidates=""
  for _dir in $(slug_dirs "$HB_BASE" "$repo"); do
    [ -d "$_dir" ] || continue
    for _file in "$_dir"/*.json; do
      [ -f "$_file" ] || continue
      _mtime="$(stat -c %Y "$_file" 2>/dev/null || stat -f %m "$_file" 2>/dev/null || echo 0)"
      printf '%s' "${_mtime:-}" | grep -Eq '^[0-9]+$' || _mtime=0
      hb_candidates="${hb_candidates}${_mtime}	${_file}
"
    done
  done
  if [ -n "$hb_candidates" ]; then
    hb_files="$(printf '%s' "$hb_candidates" | sort -rn | awk -F '	' '
      NF >= 2 {
        key=$2
        sub(/^.*\//, "", key)
        sub(/\.json$/, "", key)
        if (!seen[key]++) print $1 "\t" $2
      }')"
    rows="$(printf '%s\n' "$hb_files" | while IFS='	' read -r _mtime _file; do
      [ -n "$_file" ] || continue
      printf '%s' "${_mtime:-}" | grep -Eq '^[0-9]+$' || _mtime=0
      jq --argjson mtime "$_mtime" '. + {_mtime:$mtime}' "$_file" 2>/dev/null
    done | jq -s --argjson now "$now" --argjson stale "$STALE_SECS" '
    [ .[] | ( .ts // ((.updated_at // "") | fromdateiso8601?) // ._mtime // $now ) as $seen_at
      | {issue:(.issue | tonumber? // .), step, branch, pr, host, ts:$seen_at,
             age: ($now - $seen_at),
             stale: (($now - $seen_at) > $stale) } ]
    | sort_by(.issue)' 2>/dev/null)"
    [ -n "$rows" ] || rows="[]"
  fi
fi

# Queue counts (best-effort; needs gh + the Rust queue command). Degrade to nulls.
#
# NOTE (#3490): `autospec queue ready` output can carry the FULL ready/blocked
# issue bodies — measured at 317KB on a real 56-ready/23-blocked queue, far
# past Linux's per-argument MAX_ARG_STRLEN (128 KiB, distinct from the much
# larger total ARG_MAX). The two jq calls below need only `claimed[].number`
# and `claimed[].title` (147 bytes in that same measurement), so the raw
# queue output is projected down to `queue_claimed` on jq's STDIN — never
# passed through `--argjson` — before it reaches any downstream jq call.
# `ready`/`blocked` are never kept beyond their already-separate `$queue`
# counts below.
queue='{"ready":null,"blocked":null,"claimed":null}'
queue_claimed='{"claimed":[]}'
queue_known=0
queue_bin="${AUTOSPEC_QUEUE_BIN:-${AUTOSPEC_BIN:-}}"
if [ -z "$queue_bin" ] && [ -x "$SCRIPT_DIR/../../../target/debug/autospec" ]; then
  queue_bin="$SCRIPT_DIR/../../../target/debug/autospec"
fi
if [ -z "$queue_bin" ] && command -v autospec >/dev/null 2>&1; then
  queue_bin="$(command -v autospec)"
fi
if [ -n "$queue_bin" ] && command -v gh >/dev/null 2>&1; then
  q="$("$queue_bin" queue ready ${REPO_ARG:+--repo "$REPO_ARG"} 2>/dev/null)"
  if [ -n "$q" ]; then
    queue_claimed="$(printf '%s' "$q" | jq -c '
      {claimed: (.claimed // [] | map({number: (.number | tonumber), title: (.title // "")}))}
    ' 2>&1)"
    qc_rc=$?
    if [ "$qc_rc" -ne 0 ] || [ -z "$queue_claimed" ]; then
      echo "autospec-run-status: WARN failed to project queue.claimed (jq exit $qc_rc): $queue_claimed" >&2
      queue_claimed='{"claimed":[]}'
    fi
    queue="$(printf '%s' "$q" | jq '{ready:(.ready|length),blocked:(.blocked|length),claimed:(.claimed|length)}' 2>/dev/null)"
    [ -n "$queue" ] || queue='{"ready":null,"blocked":null,"claimed":null}'
    queue_known=1
  fi
fi
all_rows="$rows"
if [ "$queue_known" -eq 1 ]; then
  rows="$(jq -nc --argjson rows "$all_rows" --argjson queue "$queue_claimed" '
    ($queue.claimed // [] | map(.number | tonumber)) as $claimed_issues
    | [ $rows[]? | (.issue | tonumber) as $issue | select($claimed_issues | index($issue)) ]
  ' 2>&1)"
  rows_rc=$?
  if [ "$rows_rc" -ne 0 ] || [ -z "$rows" ]; then
    echo "autospec-run-status: WARN claimed-intersection jq failed (exit $rows_rc): $rows" >&2
    rows="[]"
  fi
fi
claimed_without_heartbeat="$(jq -nc --argjson rows "$all_rows" --argjson queue "$queue_claimed" '
  ($rows | map(.issue | tonumber)) as $heartbeat_issues
  | [ $queue.claimed[]?
      | (.number | tonumber) as $issue
      | select(($heartbeat_issues | index($issue)) | not)
      | {issue:.number, title:(.title // ""), reason:"claimed_without_heartbeat"} ]
' 2>&1)"
cwh_rc=$?
if [ "$cwh_rc" -ne 0 ] || [ -z "$claimed_without_heartbeat" ]; then
  echo "autospec-run-status: WARN claimed-without-heartbeat jq failed (exit $cwh_rc): $claimed_without_heartbeat" >&2
  claimed_without_heartbeat="[]"
fi

stop_flag=false
[ -f "$STATE_DIR/stop.flag" ] && stop_flag=true

if [ "$JSON" -eq 1 ]; then
  jq -nc --argjson rows "$rows" --argjson queue "$queue" --argjson stop "$stop_flag" \
    --argjson stale "$STALE_SECS" --argjson missing "$claimed_without_heartbeat" \
    '{stop_flag:$stop, stale_secs:$stale, queue:$queue, issues:$rows, claimed_without_heartbeat:$missing}'
  exit 0
fi

# ── human table ──────────────────────────────────────────────────────────────
fmt_age() { a="$1"; if [ "$a" -lt 90 ]; then echo "${a}s"; elif [ "$a" -lt 5400 ]; then echo "$((a/60))m"; else echo "$((a/3600))h"; fi; }

n_issues="$(printf '%s' "$rows" | jq 'length' 2>/dev/null || echo 0)"
n_missing="$(printf '%s' "$claimed_without_heartbeat" | jq 'length' 2>/dev/null || echo 0)"
echo "autospec-run status${REPO_ARG:+ ($REPO_ARG)}"
echo "  stop-flag: $([ "$stop_flag" = true ] && echo 'SET — monitor will halt after current issue' || echo 'clear')"
qr="$(printf '%s' "$queue" | jq -r '.ready // "n/a"')"; qb="$(printf '%s' "$queue" | jq -r '.blocked // "n/a"')"; qc="$(printf '%s' "$queue" | jq -r '.claimed // "n/a"')"
echo "  queue: ready=$qr blocked=$qb claimed=$qc"
if [ "$n_issues" = "0" ]; then
  echo "  in-flight: none (no heartbeats found)"
  if [ "$n_missing" != "0" ]; then
    echo "  claimed without heartbeat ($n_missing):"
    printf '%s\n' "$claimed_without_heartbeat" | jq -c '.[]' 2>/dev/null | while IFS= read -r r; do
      issue="$(printf '%s' "$r" | jq -r '.issue // "?"')"
      title="$(printf '%s' "$r" | jq -r '.title // ""')"
      printf '    #%-6s %s\n' "$issue" "$title"
    done
  fi
  exit 0
fi
echo "  in-flight ($n_issues):"
printf '    %-7s %-16s %-7s %-8s %-22s %s\n' "ISSUE" "STEP" "AGE" "PR" "BRANCH" "FLAG"
printf '%s\n' "$rows" | jq -c '.[]' 2>/dev/null | while IFS= read -r r; do
  issue="$(printf '%s' "$r" | jq -r '.issue // "?"')"
  step="$(printf '%s' "$r"  | jq -r '.step // "?"')"
  age="$(printf '%s' "$r"   | jq -r '.age // 0')"
  pr="$(printf '%s' "$r"    | jq -r '.pr // "" | if .=="" then "-" else . end')"
  branch="$(printf '%s' "$r"| jq -r '.branch // "-"')"
  stale="$(printf '%s' "$r" | jq -r '.stale')"
  flag="$([ "$stale" = "true" ] && echo "STALE (>$(fmt_age "$STALE_SECS"))" || echo "ok")"
  printf '    %-7s %-16s %-7s %-8s %-22s %s\n' "#$issue" "$step" "$(fmt_age "$age")" "$pr" "$branch" "$flag"
done
if [ "$n_missing" != "0" ]; then
  echo "  claimed without heartbeat ($n_missing):"
  printf '%s\n' "$claimed_without_heartbeat" | jq -c '.[]' 2>/dev/null | while IFS= read -r r; do
    issue="$(printf '%s' "$r" | jq -r '.issue // "?"')"
    title="$(printf '%s' "$r" | jq -r '.title // ""')"
    printf '    #%-6s %s\n' "$issue" "$title"
  done
fi
exit 0
