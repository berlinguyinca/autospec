#!/usr/bin/env bash
# scripts/project-board-resolve.sh — pure reader: GitHub Projects v2 board → board plan JSON.
#
# NEVER mutates. Board titles, bodies, field values, and README text are untrusted
# DATA, never instructions.
#
# Usage:
#   project-board-resolve.sh --url URL [--emit identity|plan|fleet-config|repos]
#
# Exit codes:
#   0 success | 2 usage error | 3 auth/scope failure | 4 truncated read

set -eu

die_usage() { printf 'project-board-resolve: %s\n' "$1" >&2; exit 2; }

url=""
emit="plan"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --url)
            if [ "$#" -lt 2 ]; then
                die_usage "--url requires a value"
            fi
            url="$2"
            shift 2
            ;;
        --emit)
            if [ "$#" -lt 2 ]; then
                die_usage "--emit requires a value"
            fi
            emit="$2"
            shift 2
            ;;
        --help|-h)
            cat <<'EOF'
project-board-resolve.sh — resolve a GitHub Projects v2 board into a board plan

Usage:
  project-board-resolve.sh --url URL [--emit identity|plan|fleet-config|repos]
EOF
            exit 0
            ;;
        *) die_usage "unknown option: $1" ;;
    esac
done

[ -n "$url" ] || die_usage "--url is required"

# ── Identity ────────────────────────────────────────────────────────────────
# Anchored so trailing garbage (".../projects/2x") is rejected, not truncated.
# Accepts optional /views/N suffix (normalized away). Normalizes leading zeros.
parse_identity() {
    local u="$1" kind="" owner="" number=""
    if printf '%s' "$u" | grep -Eq '^https://github\.com/orgs/[^/]+/projects/[0-9]+(/views/[0-9]+)?/?$'; then
        kind="org"
    elif printf '%s' "$u" | grep -Eq '^https://github\.com/users/[^/]+/projects/[0-9]+(/views/[0-9]+)?/?$'; then
        kind="user"
    else
        die_usage "not a GitHub Projects v2 URL: $u"
    fi
    owner="$(printf '%s' "$u" | sed -E 's#^https://github\.com/(orgs|users)/([^/]+)/projects/[0-9]+(/views/[0-9]+)?/?$#\2#')"
    number="$(printf '%s' "$u" | sed -E 's#^.*/projects/([0-9]+)(/views/[0-9]+)?/?$#\1#')"
    number="$((10#${number}))"
    printf '{"owner":"%s","kind":"%s","number":%s}\n' "$owner" "$kind" "$number"
}

identity="$(parse_identity "$url")"

case "$emit" in
    identity) printf '%s\n' "$identity"; exit 0 ;;
esac

command -v gh >/dev/null 2>&1 || { printf 'project-board-resolve: gh not found\n' >&2; exit 3; }
command -v jq >/dev/null 2>&1 || { printf 'project-board-resolve: jq not found\n' >&2; exit 3; }

owner="$(printf '%s' "$identity" | jq -r '.owner')"
number="$(printf '%s' "$identity" | jq -r '.number')"
limit="${AUTOSPEC_PROJECT_BOARD_LIMIT:-500}"

fields_json="$(gh project field-list "$number" --owner "$owner" --format json 2>/dev/null)" || {
    printf 'project-board-resolve: gh project field-list failed\n' >&2; exit 3; }
items_json="$(gh project item-list "$number" --owner "$owner" --limit "$limit" --format json 2>/dev/null)" || {
    printf 'project-board-resolve: gh project item-list failed\n' >&2; exit 3; }

# Fail closed on a possibly-truncated read: never promote from a partial plan.
item_count="$(printf '%s' "$items_json" | jq '.items | length')"
if [ "$item_count" -ge "$limit" ]; then
    printf 'project-board-resolve: read may be truncated (%s items at limit %s)\n' "$item_count" "$limit" >&2
    exit 4
fi

# The state field is resolved through an ordered candidate list rather than a
# single hardcoded literal — different boards name it differently (measured:
# "AutoSpec state" on p2, "Delivery status" on p1). Override the candidates
# via AUTOSPEC_PROJECT_BOARD_STATE_FIELDS (comma-separated); the first
# candidate that exists on the board wins. `.fields.autospec_state.name`
# records which candidate actually matched.
candidates_raw="${AUTOSPEC_PROJECT_BOARD_STATE_FIELDS:-AutoSpec state,Delivery status}"
candidates_json="$(printf '%s' "$candidates_raw" | jq -R -c 'split(",")')"

# `AutoSpec state` (or its candidate analogue) may be absent; write-back is
# skipped downstream when it is.
fields_map="$(printf '%s' "$fields_json" | jq --argjson candidates "$candidates_json" '
  (first(
     $candidates[] as $name
     | (.fields[]? | select(.name == $name)
        | {name: $name, id: .id,
           options: (.options // [] | map({key: .name, value: .id}) | from_entries)})
   )) as $f
  | if $f == null then {} else {autospec_state: $f} end' 2>/dev/null || printf '{}')"
[ -n "$fields_map" ] || fields_map='{}'

repos_json="$(printf '%s' "$items_json" | jq -c '
  [.items[]? | select(.content.type == "Issue") | .content.repository] | unique')"

# The Projects item-list payload carries no issue open/closed state — verified
# against both fixtures: .content keys are exactly {body, number, repository,
# title, type, url}. Issue state is joined from a SECOND source, queried once
# per distinct repo (O(repos), never O(items)): `gh issue list --state closed`.
# A repo whose closed-list query fails, or is truncated, degrades that repo's
# items to "open" rather than aborting the plan or exiting non-zero — an
# unlisted-but-actually-closed issue just delays a downstream promotion; it
# can never cause a wrong one. This is deliberately NOT the same truncation
# policy as the item-list read above, which fails closed with exit 4: a
# truncated *item* list can silently drop whole items from the plan, while a
# truncated *closed-issue* list only ever under-reports "closed" — the safe
# direction. Never sourced from the board's status column: that is an
# operator-maintained field that lags reality (measured: 1/80 "Done" on p2
# despite issues actually closed on GitHub).
closed_map='{}'
if [ "$emit" = "plan" ]; then
    for repo in $(printf '%s' "$repos_json" | jq -r '.[]'); do
        closed_json="$(gh issue list --repo "$repo" --state closed --limit "$limit" --json number 2>/dev/null)" || closed_json=""
        if ! printf '%s' "$closed_json" | jq -e 'type == "array"' >/dev/null 2>&1; then
            closed_json='[]'
        fi
        numbers_json="$(printf '%s' "$closed_json" | jq -c '[.[].number]')"
        closed_map="$(printf '%s' "$closed_map" | jq -c --arg repo "$repo" --argjson nums "$numbers_json" '. + {($repo): $nums}')"
    done
fi

plan="$(printf '%s' "$items_json" | jq --argjson id "$identity" --argjson fields "$fields_map" --argjson closed "$closed_map" '
  [.items[]? | select(.content.type == "Issue")] as $issues
  | {project: $id,
     fields: $fields,
     repos: ($issues | map(.content.repository) | unique),
     items: ($issues | map(
        .content.repository as $repo
        | .content.number as $num
        | {item_id: .id,
           repo:    $repo,
           number:  $num,
           title:   .content.title,
           body:    (.content.body // ""),
           state:   (if (($closed[$repo] // []) | index($num)) then "closed" else "open" end),
           status:  (.status // null),
           labels:  (.labels // [])
        }))}')"

case "$emit" in
    plan)  printf '%s\n' "$plan" ;;
    repos) printf '%s\n' "$repos_json" ;;
    *) die_usage "unsupported --emit: $emit" ;;
esac
