#!/usr/bin/env bash
# resume-attempts.sh — durable consecutive-auto-resume-attempt counter.
#
# Per docs/specs/2026-06-03-crash-resume-design.md (§Data model, §Error
# handling): a bounded number of consecutive auto-resume attempts without
# forward progress (any issue reaching `merged`) before resume halts and
# surfaces to the operator — never an infinite boot-thrash loop.
#
# Counter file (path-scoped by repo-slug):
#   ~/.autospec/resume-attempts/<repo-slug>.json
#   { "schema": 1, "repo": "owner/name", "count": 0,
#     "first_at": "<iso8601>", "updated_at": "<iso8601>" }
#
# Two-line atomic temp+mv write. A counter whose updated_at is older than 24h is
# considered stale and treated as count=0 (sentinel convention) — a crash days
# ago must not permanently cap recovery.
#
# Subcommands:
#   get    --repo <o/n>    # print current effective count (0 if absent/stale)
#   inc    --repo <o/n>    # increment and print the new count
#   reset  --repo <o/n>    # reset to 0 (forward progress; any issue merged)
#   cap                    # print the configured cap (AUTOSPEC_RESUME_MAX_ATTEMPTS, default 3)
#   at-cap --repo <o/n>    # exit 0 when effective count >= cap, else exit 1
#   path   --repo <o/n>    # print the resolved counter file path
#
# Environment:
#   AUTOSPEC_RESUME_ATTEMPTS_DIR   base dir (default: ~/.autospec/resume-attempts)
#   AUTOSPEC_RESUME_MAX_ATTEMPTS   cap (default: 3)

set -eu

ATTEMPTS_BASE="${AUTOSPEC_RESUME_ATTEMPTS_DIR:-$HOME/.autospec/resume-attempts}"
STALE_SECS="${AUTOSPEC_RESUME_ATTEMPTS_STALE_SECS:-86400}"   # 24h

err() { printf 'resume-attempts: %s\n' "$1" >&2; }
die() { err "$1"; exit 1; }

usage() {
    cat <<'EOF'
Usage:
  resume-attempts.sh get    --repo <owner/name>
  resume-attempts.sh inc    --repo <owner/name>
  resume-attempts.sh reset  --repo <owner/name>
  resume-attempts.sh cap
  resume-attempts.sh at-cap --repo <owner/name>
  resume-attempts.sh path   --repo <owner/name>
EOF
}

# Source the canonical repo-slug helper (F4). Reader and writer share this one
# repo_slug(), so routing it through canonical_slug migrates both atomically.
_RA_SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
for _rs_cand in \
    "${AUTOSPEC_REPO_SLUG_SH:-}" \
    "${_RA_SELF_DIR}/repo-slug.sh" \
    "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/repo-slug.sh" \
    "${_RA_SELF_DIR}/../../../scripts/repo-slug.sh"; do
    if [ -n "$_rs_cand" ] && [ -f "$_rs_cand" ]; then
        # shellcheck source=/dev/null
        . "$_rs_cand"
        break
    fi
done

repo_slug() {
    repo="${1:-}"
    [ -n "$repo" ] || die "repo is required"
    case "$repo" in
        */*)
            if command -v canonical_slug >/dev/null 2>&1; then
                canonical_slug "$repo"
            else
                printf '%s' "$repo" | sed 's#/#__#'
            fi
            ;;
        *) printf '%s' "$repo" ;;   # slashless input has no canonical form
    esac
}

attempts_path() {
    slug="$(repo_slug "$1")"
    printf '%s/%s.json' "$ATTEMPTS_BASE" "$slug"
}

now_iso() { date -u +'%Y-%m-%dT%H:%M:%SZ'; }

iso_to_epoch() {
    ts="$1"
    [ -n "$ts" ] || { echo 0; return; }
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$ts" +%s 2>/dev/null \
        || date -u -d "$ts" +%s 2>/dev/null \
        || echo 0
}

cap_value() {
    cap="${AUTOSPEC_RESUME_MAX_ATTEMPTS:-3}"
    case "$cap" in ''|*[!0-9]*) cap=3 ;; esac
    printf '%s' "$cap"
}

# effective_count FILE -> echo the count, treating stale (>24h) or
# missing/unparseable files as 0.
effective_count() {
    file="$1"
    [ -f "$file" ] || { echo 0; return; }
    updated="$(jq -r '.updated_at // empty' "$file" 2>/dev/null || true)"
    epoch="$(iso_to_epoch "$updated")"
    if [ "$epoch" -gt 0 ]; then
        now="$(date -u +%s)"
        age=$((now - epoch))
        if [ "$age" -ge "$STALE_SECS" ]; then
            echo 0; return
        fi
    fi
    count="$(jq -r '.count // 0' "$file" 2>/dev/null || echo 0)"
    case "$count" in ''|*[!0-9]*) count=0 ;; esac
    printf '%s' "$count"
}

write_counter() {
    repo="$1"; count="$2"
    path="$(attempts_path "$repo")"
    mkdir -p "$ATTEMPTS_BASE"
    now="$(now_iso)"
    first_at="$now"
    if [ -f "$path" ]; then
        prev="$(jq -r '.first_at // empty' "$path" 2>/dev/null || true)"
        [ -n "$prev" ] && first_at="$prev"
    fi
    tmp="$(mktemp "${path}.XXXXXX")"
    if ! jq -n \
        --arg repo "$repo" \
        --argjson count "$count" \
        --arg first_at "$first_at" \
        --arg updated_at "$now" \
        '{schema:1, repo:$repo, count:$count, first_at:$first_at, updated_at:$updated_at}' \
        > "$tmp"; then
        rm -f "$tmp"
        die "failed to build counter JSON"
    fi
    mv "$tmp" "$path"
}

require_repo() {
    repo=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --repo) repo="${2:-}"; shift 2 ;;
            *) die "unknown option: $1" ;;
        esac
    done
    [ -n "$repo" ] || die "--repo is required"
    printf '%s' "$repo"
}

subcommand="${1:-}"
[ -n "$subcommand" ] || { usage >&2; exit 1; }
shift || true

case "$subcommand" in
    get)
        repo="$(require_repo "$@")"
        printf '%s\n' "$(effective_count "$(attempts_path "$repo")")"
        ;;
    inc)
        repo="$(require_repo "$@")"
        cur="$(effective_count "$(attempts_path "$repo")")"
        new=$((cur + 1))
        write_counter "$repo" "$new"
        printf '%s\n' "$new"
        ;;
    reset)
        repo="$(require_repo "$@")"
        write_counter "$repo" 0
        printf '0\n'
        ;;
    cap)
        printf '%s\n' "$(cap_value)"
        ;;
    at-cap)
        repo="$(require_repo "$@")"
        cur="$(effective_count "$(attempts_path "$repo")")"
        cap="$(cap_value)"
        if [ "$cur" -ge "$cap" ]; then exit 0; else exit 1; fi
        ;;
    path)
        repo="$(require_repo "$@")"
        attempts_path "$repo"
        ;;
    --help|-h) usage ;;
    *) usage >&2; exit 1 ;;
esac
