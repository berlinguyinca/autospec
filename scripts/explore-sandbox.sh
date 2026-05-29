#!/usr/bin/env bash
# scripts/explore-sandbox.sh — create or reuse the autospec-explore sandbox branch.
#
# Creates `autospec/explore/<YYYY-MM-DD>-<slug>` off `origin/<base>` if not
# present, pushes the branch to `origin`, and writes `.autospec/explore-mode.json`
# with `{branch, slug, base, head_sha, created_at}`.
#
# Idempotent: re-invocation with the same `--slug` reuses the existing sandbox
# branch — no error, no duplicate, no force-push. The script verifies the existing
# branch's recorded base matches `--base`; mismatch exits 3 with
# `code_health:explore_sandbox_base_mismatch`.
#
# Usage:
#   scripts/explore-sandbox.sh [--slug <slug>] [--base <branch>] [--dry-run]
#
# Defaults:
#   --slug   ISO-date + 6-char random suffix
#   --base   main

set -euo pipefail

SLUG=""
BASE="main"
DRY_RUN=0

usage() {
    cat <<EOF
Usage: $0 [--slug <slug>] [--base <branch>] [--dry-run]

Creates or reuses an autospec-explore sandbox branch off origin/<base>,
pushes it, and writes .autospec/explore-mode.json.

Idempotent: re-invocation with the same slug reuses the existing branch.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --slug)        shift; SLUG="${1:-}" ;;
        --slug=*)      SLUG="${1#--slug=}" ;;
        --base)        shift; BASE="${1:-main}" ;;
        --base=*)      BASE="${1#--base=}" ;;
        --dry-run)     DRY_RUN=1 ;;
        -h|--help)     usage; exit 0 ;;
        *) printf 'error: unknown arg: %s\n' "$1" >&2; usage; exit 2 ;;
    esac
    shift
done

DATE_PREFIX="$(date -u +%Y-%m-%d)"

if [ -z "$SLUG" ]; then
    RAND="$(LC_ALL=C tr -dc 'a-z0-9' < /dev/urandom 2>/dev/null | head -c 6 || echo "$$$(date +%s)" | tr -dc 'a-z0-9' | head -c 6)"
    SLUG="auto-${RAND}"
fi

BRANCH="autospec/explore/${DATE_PREFIX}-${SLUG}"
MODE_FILE=".autospec/explore-mode.json"
CREATED_AT="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"

run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '  [dry-run] %s\n' "$*"
    else
        eval "$@"
    fi
}

info() { printf '%s\n' "$*"; }
err()  { printf 'error: %s\n' "$*" >&2; }

# Repo sanity
if ! git rev-parse --show-toplevel >/dev/null 2>&1; then
    err "not inside a git repository"
    exit 2
fi

# Fetch the base so we can branch off the remote tip
run "git fetch origin \"$BASE\" --quiet || true"

# Idempotency check: if .autospec/explore-mode.json already records this slug,
# verify the base matches and just refresh head_sha.
if [ -f "$MODE_FILE" ]; then
    EXISTING_SLUG="$(grep -o '"slug"[[:space:]]*:[[:space:]]*"[^"]*"' "$MODE_FILE" | sed 's/.*"slug"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')"
    EXISTING_BASE="$(grep -o '"base"[[:space:]]*:[[:space:]]*"[^"]*"' "$MODE_FILE" | sed 's/.*"base"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')"
    if [ "$EXISTING_SLUG" = "$SLUG" ] && [ -n "$EXISTING_BASE" ] && [ "$EXISTING_BASE" != "$BASE" ]; then
        err "code_health:explore_sandbox_base_mismatch — slug=$SLUG existing base=$EXISTING_BASE requested base=$BASE"
        exit 3
    fi
fi

# Does the branch already exist locally or on origin?
BRANCH_EXISTS_LOCAL=0
BRANCH_EXISTS_REMOTE=0
if git show-ref --verify --quiet "refs/heads/$BRANCH"; then BRANCH_EXISTS_LOCAL=1; fi
if git ls-remote --exit-code --heads origin "$BRANCH" >/dev/null 2>&1; then BRANCH_EXISTS_REMOTE=1; fi

if [ "$BRANCH_EXISTS_LOCAL" -eq 1 ] || [ "$BRANCH_EXISTS_REMOTE" -eq 1 ]; then
    info "sandbox branch already exists: $BRANCH (reusing — idempotent)"
    if [ "$BRANCH_EXISTS_LOCAL" -eq 0 ] && [ "$BRANCH_EXISTS_REMOTE" -eq 1 ]; then
        run "git fetch origin \"$BRANCH:$BRANCH\" --quiet"
    fi
else
    info "creating sandbox branch: $BRANCH off origin/$BASE"
    run "git branch \"$BRANCH\" \"origin/$BASE\""
    run "git push -u origin \"$BRANCH\""
fi

# Capture the head SHA of the sandbox branch (post-create or pre-existing).
if [ "$DRY_RUN" -eq 1 ]; then
    HEAD_SHA="dryrun0000000000000000000000000000000000"
else
    HEAD_SHA="$(git rev-parse "$BRANCH")"
fi

# Write .autospec/explore-mode.json
run "mkdir -p .autospec"
JSON_PAYLOAD=$(printf '{\n  "branch": "%s",\n  "slug": "%s",\n  "base": "%s",\n  "head_sha": "%s",\n  "created_at": "%s"\n}\n' \
    "$BRANCH" "$SLUG" "$BASE" "$HEAD_SHA" "$CREATED_AT")

if [ "$DRY_RUN" -eq 1 ]; then
    printf '  [dry-run] write %s:\n%s\n' "$MODE_FILE" "$JSON_PAYLOAD"
else
    printf '%s' "$JSON_PAYLOAD" > "$MODE_FILE"
    info "wrote $MODE_FILE"
fi

info ""
info "Sandbox ready:"
info "  branch:   $BRANCH"
info "  base:     $BASE"
info "  head_sha: $HEAD_SHA"
info ""
info "All PRs from /autospec-explore will target this branch — main is never written to directly."

exit 0
