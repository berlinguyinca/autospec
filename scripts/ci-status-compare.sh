#!/usr/bin/env bash
# ci-status-compare.sh — classify failed PR checks as inherited base rot or branch-caused.
#
# Usage:
#   bash scripts/ci-status-compare.sh --head head-rollup.json --base base-rollup.json
#
# Inputs are JSON arrays shaped like gh statusCheckRollup output. The helper is
# intentionally file-based so callers can capture head/base evidence with gh and
# then make a deterministic, reproducible merge-gate decision from those files.
#
# Output JSON:
#   {
#     "classification": "mergeable" | "inherited" | "branch_caused",
#     "target_url": "https://..." | null,
#     "blocked_branch": [...],
#     "blocked_inherited": [...]
#   }
set -euo pipefail

usage() {
    cat >&2 <<'USAGE'
Usage: ci-status-compare.sh --head <head-check-rollup.json> --base <base-check-rollup.json>

Classifies failed head checks by comparing them with base-branch check JSON.
USAGE
}

HEAD_JSON=""
BASE_JSON=""

while [ $# -gt 0 ]; do
    case "$1" in
        --head)
            [ $# -ge 2 ] || { usage; exit 2; }
            HEAD_JSON="$2"
            shift 2
            ;;
        --base)
            [ $# -ge 2 ] || { usage; exit 2; }
            BASE_JSON="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            printf 'ci-status-compare.sh: unknown option: %s\n' "$1" >&2
            usage
            exit 2
            ;;
    esac
done

if [ -z "$HEAD_JSON" ] || [ -z "$BASE_JSON" ]; then
    usage
    exit 2
fi
if [ ! -f "$HEAD_JSON" ]; then
    printf 'ci-status-compare.sh: missing --head file: %s\n' "$HEAD_JSON" >&2
    exit 2
fi
if [ ! -f "$BASE_JSON" ]; then
    printf 'ci-status-compare.sh: missing --base file: %s\n' "$BASE_JSON" >&2
    exit 2
fi

jq -n --slurpfile head "$HEAD_JSON" --slurpfile base "$BASE_JSON" '
  def rollup($x):
    if ($x | type) == "array" then $x
    elif ($x.statusCheckRollup? | type) == "array" then $x.statusCheckRollup
    elif ($x.checks? | type) == "array" then $x.checks
    else [] end;

  def check_name:
    (.name // .context // .workflowName // .displayName // .title // "unknown");

  def check_url:
    (.detailsUrl // .targetUrl // .target_url // .url // .htmlUrl // .link // null);

  def normalized:
    {
      name: check_name,
      key: (check_name | ascii_downcase),
      conclusion: ((.conclusion // .state // .status // "UNKNOWN") | tostring | ascii_upcase),
      status: ((.status // "") | tostring | ascii_upcase),
      target_url: check_url
    };

  def bad_conclusion:
    .conclusion as $c
    | ($c == "FAILURE" or $c == "FAILED" or $c == "ERROR" or $c == "CANCELLED" or $c == "TIMED_OUT" or $c == "ACTION_REQUIRED");

  (rollup($head[0]) | map(normalized)) as $head_checks
  | (rollup($base[0]) | map(normalized)) as $base_checks
  | ($head_checks | map(select(bad_conclusion))) as $head_bad
  | ($base_checks | map(select(bad_conclusion)) | map({key, value: .}) | from_entries) as $base_bad_by_key
  | ($head_bad | map(. + {base_conclusion: ($base_bad_by_key[.key].conclusion // null)})) as $annotated_bad
  | ($annotated_bad | map(select(.base_conclusion == null))) as $blocked_branch
  | ($annotated_bad | map(select(.base_conclusion != null))) as $blocked_inherited
  | {
      classification:
        (if ($blocked_branch | length) > 0 then "branch_caused"
         elif ($blocked_inherited | length) > 0 then "inherited"
         else "mergeable" end),
      target_url: ($annotated_bad[0].target_url // null),
      blocked_branch: ($blocked_branch | map(del(.key))),
      blocked_inherited: ($blocked_inherited | map(del(.key)))
    }
' 
