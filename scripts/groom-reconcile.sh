#!/usr/bin/env bash
# scripts/groom-reconcile.sh — stamp real clean-merge outcomes into the grooming
# telemetry log from GitHub. For each record with outcome==null whose issue is
# now closed, query gh and set closing_pr + outcome. Bounded to unresolved
# records; a gh failure leaves the record unresolved (never counted clean).
#
# `clean` is provably positive ONLY: CLOSED + stateReason COMPLETED + >=1 closing
# PR + no escalate:human + no groom:rejected. escalate/rejected take precedence.
#
# Usage: groom-reconcile.sh --telemetry <jsonl> --repo <owner/name>
# Exit: always 0 (never blocks the sweep).
set -eu

GH_BIN="${AUTOSPEC_GH_BIN:-gh}"
TELE="" REPO=""
while [ $# -gt 0 ]; do
  case "$1" in
    --telemetry) TELE="${2:-}"; shift 2 ;;
    --repo) REPO="${2:-}"; shift 2 ;;
    --help|-h) printf 'Usage: groom-reconcile.sh --telemetry <jsonl> --repo <owner/name>\n'; exit 0 ;;
    *) printf 'groom-reconcile.sh: unknown option: %s\n' "$1" >&2; exit 2 ;;
  esac
done
[ -n "$TELE" ] && [ -f "$TELE" ] || exit 0
[ -n "$REPO" ] || exit 0

# Classify one gh JSON blob → outcome string, or empty to leave unresolved.
classify() {
  printf '%s' "$1" | jq -r '
    def has_label($n): ((.labels // []) | map(.name) | index($n)) != null;
    # Deferred-until-close by design: reconcile stamps *merge* outcomes, which only
    # exist once an issue closes; an OPEN issue (even escalate:human-labeled)
    # contributes no sample yet and is classified when it closes — not a missed escalate.
    if .state != "CLOSED" then ""
    elif has_label("escalate:human") then "escalate"
    elif has_label("groom:rejected") then "rejected"
    elif (.stateReason // "") != "COMPLETED" then "rejected"
    elif ((.closedByPullRequestsReferences // []) | length) == 0 then "rejected"
    else "clean" end'
}
closing_pr_of() {
  printf '%s' "$1" | jq -r '(.closedByPullRequestsReferences // [] | .[0].number) // "null"'
}

tmp="$(mktemp "${TELE}.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
# Process line-by-line; malformed lines are passed through unchanged.
while IFS= read -r line || [ -n "$line" ]; do
  if [ -z "$line" ]; then continue; fi
  # Only touch well-formed records with outcome==null.
  need="$(printf '%s' "$line" | jq -r 'if (type=="object" and (.outcome==null) and (.issue!=null)) then .issue else "skip" end' 2>/dev/null || printf 'skip')"
  if [ "$need" = "skip" ] || [ -z "$need" ]; then
    printf '%s\n' "$line" >> "$tmp"; continue
  fi
  set +e
  blob="$("$GH_BIN" issue view "$need" --repo "$REPO" \
            --json state,stateReason,closedByPullRequestsReferences,labels 2>/dev/null)"
  ghrc=$?
  set -e
  if [ "$ghrc" -ne 0 ] || [ -z "$blob" ]; then
    printf '%s\n' "$line" >> "$tmp"; continue   # fail-closed: unresolved
  fi
  set +e
  oc="$(classify "$blob")"
  ocrc=$?
  set -e
  if [ "$ocrc" -ne 0 ]; then
    printf '%s\n' "$line" >> "$tmp"; continue   # jq parse error on non-JSON gh output: unresolved
  fi
  if [ -z "$oc" ]; then
    printf '%s\n' "$line" >> "$tmp"; continue   # still open → unresolved
  fi
  set +e
  pr="$(closing_pr_of "$blob")"
  prrc=$?
  set -e
  if [ "$prrc" -ne 0 ]; then
    printf '%s\n' "$line" >> "$tmp"; continue   # jq parse error: unresolved
  fi
  printf '%s' "$line" | jq -c --arg oc "$oc" --argjson pr "$pr" \
    '.outcome=$oc | .closing_pr=$pr' >> "$tmp"
done < "$TELE"
mv "$tmp" "$TELE"
exit 0
