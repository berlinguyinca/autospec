#!/usr/bin/env bash
# scripts/autonomous-provenance.sh — resolve whether an autonomous issue's
# origin is `self` (autospec-filed) or `operator` (human-directed), per
# docs/specs/2026-07-10-autonomous-integration-branch-design.md
# (§Architecture item 3, §Data model, §Error handling).
#
# Reuses scripts/lint-issue-safety.sh's trusted-actor comment idiom
# (`^approve(d)?\b` authored by a trusted-actor login) and mirrors
# scripts/autonomous-guardrails.sh's subcommand/arg-parse/die idiom.
#
# Provenance is durable by construction: labels + comments live on GitHub, so
# a resumed conductor re-derives the identical answer; nothing dispatch
# critical stays in memory. Every path that cannot prove `operator` fails
# closed to `self`.

set -eu

SCRIPT_NAME="autonomous-provenance"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./autospec-runtime-config.sh
. "$SCRIPT_DIR/autospec-runtime-config.sh"

usage() {
    cat <<'USAGE'
Usage:
  autonomous-provenance.sh resolve --issue N --repo OWNER/REPO

Prints exactly one of:
  self       — issue is autospec-originated (no trusted operator approval)
  operator   — issue is human-directed or trusted-actor approved

Ordered rules (first match wins):
  1. origin:self label present AND no approval marker           -> self
  2. approval marker present                                    -> operator
     (label `approved-by-operator` whose LAST `labeled` timeline
      event actor is a trusted actor; OR an issue comment matching
      ^approve(d)?\b authored by a trusted actor)
  3. no origin:self label AND issue author is a human (not a bot) -> operator
  4. anything else (unknown author, missing labels, API failure/
     ambiguity)                                                  -> self

Fails closed to `self` on any gh/API failure or ambiguity.
USAGE
}

die() { printf '%s: %s\n' "$SCRIPT_NAME" "$1" >&2; exit 2; }

is_valid_json() {
    printf '%s' "$1" | jq -e . >/dev/null 2>&1
}

# gh_api_json <path> — returns "" on any failure (network, auth, 404, or a
# non-JSON body). Callers must treat "" as "could not resolve" and fail
# closed rather than crash.
gh_api_json() {
    local path="$1" out
    out="$(gh api "$path" 2>/dev/null)" || { printf ''; return 0; }
    if is_valid_json "$out"; then
        printf '%s' "$out"
    else
        printf ''
    fi
}

# trusted_actor_logins — newline-separated logins from
# safety.issue_intent_gate.trusted_actors, or empty on missing/invalid config.
trusted_actor_logins() {
    local raw
    raw="$(autospec_runtime_config_get "safety.issue_intent_gate.trusted_actors" "[]")"
    if ! is_valid_json "$raw"; then
        return 0
    fi
    printf '%s' "$raw" | jq -r '.[]?.login // empty'
}

# is_trusted_login <login> <trusted-logins-newline-list>
is_trusted_login() {
    local login="$1" trusted="$2" cand
    [ -n "$login" ] || return 1
    while IFS= read -r cand; do
        [ -n "$cand" ] || continue
        if [ "$cand" = "$login" ]; then
            return 0
        fi
    done <<EOF_TRUSTED
$trusted
EOF_TRUSTED
    return 1
}

# has_label <issue-json> <label-name>
has_label() {
    printf '%s' "$1" | jq -e --arg l "$2" '[.labels[]?.name // empty] | any(. == $l)' >/dev/null 2>&1
}

# approval_via_label <repo> <issue> <trusted-logins> — exit 0 if the LAST
# `labeled` timeline event for `approved-by-operator` was actioned by a
# trusted login; forgery-proof against a label re-applied by an
# untrusted/bot actor.
approval_via_label() {
    local repo="$1" issue="$2" trusted="$3" timeline_json last_actor
    timeline_json="$(gh_api_json "repos/$repo/issues/$issue/timeline")"
    [ -n "$timeline_json" ] || return 1
    last_actor="$(printf '%s' "$timeline_json" | jq -r '
        [.[] | select(.event == "labeled" and .label.name == "approved-by-operator")]
        | last
        | .actor.login // empty
    ')"
    [ -n "$last_actor" ] || return 1
    is_trusted_login "$last_actor" "$trusted"
}

# approval_via_comment <repo> <issue> <trusted-logins> — exit 0 if any
# comment matching ^approve(d)?\b was authored by a trusted login. Reuses
# lint-issue-safety.sh's trusted-actor comment-parsing intent.
approval_via_comment() {
    local repo="$1" issue="$2" trusted="$3" comments_json candidate
    comments_json="$(gh_api_json "repos/$repo/issues/$issue/comments")"
    [ -n "$comments_json" ] || return 1
    while IFS= read -r candidate; do
        [ -n "$candidate" ] || continue
        if is_trusted_login "$candidate" "$trusted"; then
            return 0
        fi
    done <<EOF_CANDIDATES
$(printf '%s' "$comments_json" | jq -r '.[] | select(.body != null and (.body | test("^approve(d)?\\b"))) | .user.login // empty')
EOF_CANDIDATES
    return 1
}

cmd_resolve() {
    local issue="" repo=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --issue) issue="${2:-}"; shift 2 ;;
            --repo) repo="${2:-}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) die "resolve: unknown option: $1" ;;
        esac
    done
    [ -n "$issue" ] || die "resolve: --issue is required"
    [ -n "$repo" ] || die "resolve: --repo is required"

    local issue_json
    issue_json="$(gh_api_json "repos/$repo/issues/$issue")"
    if [ -z "$issue_json" ]; then
        printf 'self\n'
        return 0
    fi

    local origin_self=1 user_login user_type trusted
    if has_label "$issue_json" "origin:self"; then
        origin_self=0
    fi
    user_login="$(printf '%s' "$issue_json" | jq -r '.user.login // empty')"
    user_type="$(printf '%s' "$issue_json" | jq -r '.user.type // empty')"
    trusted="$(trusted_actor_logins)"

    local approved=1
    if approval_via_label "$repo" "$issue" "$trusted"; then
        approved=0
    elif approval_via_comment "$repo" "$issue" "$trusted"; then
        approved=0
    fi

    # Rule 1: origin:self present AND no approval marker -> self
    if [ "$origin_self" -eq 0 ] && [ "$approved" -ne 0 ]; then
        printf 'self\n'
        return 0
    fi
    # Rule 2: approval marker present -> operator
    if [ "$approved" -eq 0 ]; then
        printf 'operator\n'
        return 0
    fi
    # Rule 3: no origin:self label AND issue author is a human -> operator
    if [ "$origin_self" -ne 0 ] && [ "$user_type" = "User" ] && [ -n "$user_login" ]; then
        printf 'operator\n'
        return 0
    fi
    # Rule 4: fail closed
    printf 'self\n'
}

main() {
    [ "$#" -gt 0 ] || { usage >&2; exit 2; }
    case "$1" in
        -h|--help) usage; exit 0 ;;
    esac
    local sub="$1"; shift
    case "$sub" in
        resolve) cmd_resolve "$@" ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown subcommand: $sub" ;;
    esac
}

main "$@"
