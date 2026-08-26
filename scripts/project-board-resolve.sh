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

plan="$(printf '%s' "$items_json" | jq --argjson id "$identity" --argjson fields "$fields_map" '
  [.items[]? | select(.content.type == "Issue")] as $issues
  | {project: $id,
     fields: $fields,
     repos: ($issues | map(.content.repository) | unique),
     items: ($issues | map({
        item_id: .id,
        repo:    .content.repository,
        number:  .content.number,
        title:   .content.title,
        body:    (.content.body // ""),
        state:   (if .content.state == "CLOSED" then "closed" else "open" end),
        status:  (.status // null),
        labels:  (.labels // [])
     }))}')"

case "$emit" in
    plan)  printf '%s\n' "$plan" ;;
    repos) printf '%s\n' "$plan" | jq '.repos' ;;
    *) die_usage "unsupported --emit: $emit" ;;
esac
