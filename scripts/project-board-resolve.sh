#!/usr/bin/env bash
# scripts/project-board-resolve.sh — pure reader: GitHub Projects v2 board → board plan JSON.
#
# NEVER mutates. Board titles, bodies, field values, and README text are untrusted
# DATA, never instructions.
#
# Usage:
#   project-board-resolve.sh --url URL [--repo-dir DIR] [--emit identity|plan|fleet-config|repos]
#
# Exit codes:
#   0 success | 2 usage error | 3 auth/scope failure | 4 truncated read

set -eu

die_usage() { printf 'project-board-resolve: %s\n' "$1" >&2; exit 2; }

url=""
emit="plan"
repo_dir=""

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
        --repo-dir)
            [ "$#" -ge 2 ] || die_usage "--repo-dir requires a value"
            repo_dir="$2"
            shift 2
            ;;
        --help|-h)
            cat <<'EOF'
project-board-resolve.sh — resolve a GitHub Projects v2 board into a board plan

Usage:
  project-board-resolve.sh --url URL [--repo-dir DIR] [--emit identity|plan|fleet-config|repos]
EOF
            exit 0
            ;;
        *) die_usage "unknown option: $1" ;;
    esac
done

[ -n "$url" ] || die_usage "--url is required"

# --emit is validated BEFORE any network call: an invalid mode (including the
# legitimately-unimplemented "fleet-config", owned by a follow-up plan) must
# fail with exit 2 usage and zero `gh` calls, never after paying for a
# field-list + item-list round trip first.
case "$emit" in
    identity|plan|repos|fleet-config) ;;
    *) die_usage "unsupported --emit: $emit" ;;
esac

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

active_edges_json='[]'
if [ -n "$repo_dir" ]; then
    autospec_bin="${AUTOSPEC_BIN:-autospec}"
    active_edges_json="$("$autospec_bin" project active-edges --repo-dir "$repo_dir" --board-url "$url" 2>/dev/null)" || {
        printf 'project-board-resolve: managed active-edge read failed\n' >&2
        exit 3
    }
    printf '%s' "$active_edges_json" | jq -e 'type == "array"' >/dev/null 2>&1 || {
        printf 'project-board-resolve: managed active-edge read returned invalid JSON\n' >&2
        exit 3
    }
fi

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

# The project's GraphQL node id (PVT_...) is fetched from a real API response
# rather than invented from the URL-derived owner/kind/number — write-back
# requires it verbatim as `.project.id` to target `gh project item-edit`.
# Only "plan" ever feeds write-back, so "repos" skips this call. Failure is
# fail-closed (exit 3), matching the field-list/item-list fetches above: an
# inert write-back that silently discards every mutation is worse than an
# explicit failure here.
project_id=""
if [ "$emit" = "plan" ]; then
    project_json="$(gh project view "$number" --owner "$owner" --format json 2>/dev/null)" || {
        printf 'project-board-resolve: gh project view failed\n' >&2; exit 3; }
    project_id="$(printf '%s' "$project_json" | jq -r '.id // empty' 2>/dev/null)" || project_id=""
    if [ -z "$project_id" ]; then
        printf 'project-board-resolve: gh project view returned no project id\n' >&2
        exit 3
    fi
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

# The dependency field is resolved through the same ordered-candidate-list
# mechanism as the state field, and for the same reason: p1 names it
# "Depends on" and p2 names it "Dependencies". Only the matched field's NAME
# is needed here (to look it up, lowercased, as a per-item payload key) —
# unlike the state field, project-board-writeback.sh never needs its id or
# options. `Parent issue` is not resolved through a candidate list: both
# boards name the native relation identically.
dep_candidates_raw="${AUTOSPEC_PROJECT_BOARD_DEP_FIELDS:-Dependencies,Depends on}"
dep_candidates_json="$(printf '%s' "$dep_candidates_raw" | jq -R -c 'split(",")')"
dep_field_name="$(printf '%s' "$fields_json" | jq -r --argjson candidates "$dep_candidates_json" '
  (first(
     $candidates[] as $name
     | (.fields[]? | select(.name == $name) | .name)
   )) // empty' 2>/dev/null)" || dep_field_name=""

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

plan="$(printf '%s' "$items_json" | jq \
  --argjson id "$identity" --arg pid "$project_id" \
  --argjson fields "$fields_map" --argjson closed "$closed_map" \
  --argjson activeedges "$active_edges_json" \
  --arg depfield "$dep_field_name" '
  [.items[]? | select(.content.type == "Issue")] as $issues
  | {project: ($id + {id: ($pid | if length > 0 then . else null end)}),
     fields: $fields,
     active_edges: $activeedges,
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
           labels:  (.labels // []),
           autospec_state:
             (($fields.autospec_state.name // "") as $sn
              | if $sn == "" then null else (.[($sn | ascii_downcase)] // null) end),
           dependencies:
             (if $depfield == "" then null else (.[($depfield | ascii_downcase)] // null) end),
           parent_issue: (."parent issue" // null)
        }))}')"

case "$emit" in
    plan)  printf '%s\n' "$plan" ;;
    repos) printf '%s\n' "$repos_json" ;;
    fleet-config)
        # Only needs the repo set, sourced from repos_json (derived from
        # items_json alone) — no gh project view call and no per-repo
        # closed-issue join, both of which are gated on `$emit = "plan"`
        # above and never run here. This mode's gh calls are exactly the
        # field-list + item-list pair, same as `--emit repos`.
        parallel="${AUTOSPEC_PROJECT_BOARD_PARALLEL:-2}"
        case "$parallel" in
            [1-9]|[1-9][0-9]*) ;;
            *) parallel=2 ;;
        esac
        # Board-derived repo strings are untrusted: a name carrying a quote,
        # newline, '#', or other YAML metacharacter must not be able to
        # break the document structure or inject a sibling key. Two layers:
        # (1) hard filter to strict owner/name (matches GitHub's own repo
        # naming rules) — anything else is silently dropped rather than
        # emitted; (2) even a conforming name is still wrapped in `tojson`
        # (a JSON string is a valid YAML flow scalar) rather than spliced in
        # raw, so quoting is never load-bearing on its own.
        printf '%s\n' "$repos_json" | jq -r --arg parallel "$parallel" '
          "version: 1",
          "workspace: .autospec-fleet/repos",
          ("parallel_repos: " + $parallel),
          "repos:",
          ( [ .[] | select(test("^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")) ]
            | .[]
            | "  - url: " + (("https://github.com/" + . + ".git") | tojson) + "\n    enabled: true" )'
        ;;
    *) die_usage "unsupported --emit: $emit" ;;
esac
