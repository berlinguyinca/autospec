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
slug="$(printf '%s' "$repo" | tr '/' '_')"
HB_BASE="${AUTOSPEC_HEARTBEAT_DIR:-$HOME/.autospec/process-heartbeats}"
hb_dir="$HB_BASE/$slug"

# Build a JSON array of per-issue rows by reading the heartbeat files directly
# (no dependency on heartbeat-read.sh being co-installed).
rows="[]"
if [ -n "$slug" ] && [ -d "$hb_dir" ] && ls "$hb_dir"/*.json >/dev/null 2>&1; then
  rows="$(cat "$hb_dir"/*.json 2>/dev/null | jq -s --argjson now "$now" --argjson stale "$STALE_SECS" '
    [ .[] | {issue, step, branch, pr, host, ts,
             age: ($now - (.ts // $now)),
             stale: (($now - (.ts // $now)) > $stale) } ]
    | sort_by(.issue)' 2>/dev/null)"
  [ -n "$rows" ] || rows="[]"
fi

# Queue counts (best-effort; needs gh + list-ready-issues.sh, resolved
# defensively as a sibling then via AUTOSPEC_SCRIPTS_DIR). Degrade to nulls.
queue='{"ready":null,"blocked":null,"claimed":null}'
queue_bin=""
for _c in "$SCRIPT_DIR/list-ready-issues.sh" "${AUTOSPEC_SCRIPTS_DIR:-}/list-ready-issues.sh"; do
  [ -n "$_c" ] && [ -f "$_c" ] && { queue_bin="$_c"; break; }
done
if [ -n "$queue_bin" ] && command -v gh >/dev/null 2>&1; then
  q="$(bash "$queue_bin" ${REPO_ARG:+--repo "$REPO_ARG"} 2>/dev/null)"
  if [ -n "$q" ]; then
    queue="$(printf '%s' "$q" | jq '{ready:(.ready|length),blocked:(.blocked|length),claimed:(.claimed|length)}' 2>/dev/null)"
    [ -n "$queue" ] || queue='{"ready":null,"blocked":null,"claimed":null}'
  fi
fi

stop_flag=false
[ -f "$STATE_DIR/stop.flag" ] && stop_flag=true

if [ "$JSON" -eq 1 ]; then
  jq -nc --argjson rows "$rows" --argjson queue "$queue" --argjson stop "$stop_flag" \
    --argjson stale "$STALE_SECS" \
    '{stop_flag:$stop, stale_secs:$stale, queue:$queue, issues:$rows}'
  exit 0
fi

# ── human table ──────────────────────────────────────────────────────────────
fmt_age() { a="$1"; if [ "$a" -lt 90 ]; then echo "${a}s"; elif [ "$a" -lt 5400 ]; then echo "$((a/60))m"; else echo "$((a/3600))h"; fi; }

n_issues="$(printf '%s' "$rows" | jq 'length' 2>/dev/null || echo 0)"
echo "autospec-run status${REPO_ARG:+ ($REPO_ARG)}"
echo "  stop-flag: $([ "$stop_flag" = true ] && echo 'SET — monitor will halt after current issue' || echo 'clear')"
qr="$(printf '%s' "$queue" | jq -r '.ready // "n/a"')"; qb="$(printf '%s' "$queue" | jq -r '.blocked // "n/a"')"; qc="$(printf '%s' "$queue" | jq -r '.claimed // "n/a"')"
echo "  queue: ready=$qr blocked=$qb claimed=$qc"
if [ "$n_issues" = "0" ]; then
  echo "  in-flight: none (no heartbeats found)"
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
exit 0
