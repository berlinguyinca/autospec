#!/usr/bin/env bash
# run-groom-preflight.sh — one non-blocking backlog-grooming cycle for autospec-run.

set -eu

usage() {
    cat <<'USAGE'
Usage: run-groom-preflight.sh --repo owner/repo [--report FILE]

Runs one Phase 4 backlog-grooming preflight when grooming policy is auto/on.
Policy off skips the orchestrator. Grooming failures warn and exit 0 so the
normal auto-implement drain can proceed.
USAGE
}

die() {
    printf 'run-groom-preflight: %s\n' "$1" >&2
    exit 2
}

repo=""
report_file=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo) repo="${2:-}"; shift 2 ;;
        --report) report_file="${2:-}"; shift 2 ;;
        --help|-h) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

if [ -z "$repo" ]; then
    repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
fi
[ -n "$repo" ] || die "--repo is required when gh cannot infer it"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." 2>/dev/null && pwd || printf '')"
SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"

first_existing() {
    for candidate in "$@"; do
        if [ -n "$candidate" ] && [ -f "$candidate" ]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done
    return 1
}

GROOM_CONFIG="$(first_existing \
    "$SCRIPTS_DIR/grooming-config.sh" \
    "$SCRIPT_DIR/grooming-config.sh" \
    "$REPO_ROOT/skills/autospec-shared/scripts/grooming-config.sh" \
    "$REPO_ROOT/scripts/grooming-config.sh" \
    2>/dev/null || true)"
[ -n "$GROOM_CONFIG" ] || die "missing grooming-config.sh"

PROMOTE="$(first_existing \
    "$SCRIPTS_DIR/autonomous-promote-open-issues.sh" \
    "$SCRIPT_DIR/autonomous-promote-open-issues.sh" \
    "$REPO_ROOT/scripts/autonomous-promote-open-issues.sh" \
    2>/dev/null || true)"
[ -n "$PROMOTE" ] || die "missing autonomous-promote-open-issues.sh"

policy="$(bash "$GROOM_CONFIG" --key policy 2>/dev/null || printf 'auto')"
[ -n "$policy" ] || policy="auto"
case "$policy" in
    auto|on|off) ;;
    *) policy="auto" ;;
esac

append_report() {
    summary="$1"
    [ -n "$report_file" ] || return 0
    mkdir -p "$(dirname "$report_file")"
    {
        printf '\n### Backlog grooming preflight\n'
        printf '%s\n' "$summary"
    } >> "$report_file"
}

empty_summary() {
    status="$1"
    jq -c -n --arg status "$status" --arg policy "$policy" \
        '{status:$status, policy:$policy, promoted:[], "groom:proposed":[], held:[]}'
}

if [ "$policy" = "off" ]; then
    empty_summary skipped
    exit 0
fi

out_file="$(mktemp -t run-groom-preflight.out.XXXXXX)"
err_file="$(mktemp -t run-groom-preflight.err.XXXXXX)"
trap 'rm -f "$out_file" "$err_file"' EXIT

if ! bash "$PROMOTE" --repo "$repo" --apply >"$out_file" 2>"$err_file"; then
    warn="$(tr '\n' ' ' < "$err_file" | sed 's/[[:space:]][[:space:]]*/ /g; s/^ //; s/ $//')"
    [ -n "$warn" ] || warn="orchestrator exited non-zero"
    printf 'WARN: backlog grooming preflight failed: %s\n' "$warn"
    summary="$(empty_summary warn)"
    append_report "$summary"
    printf '%s\n' "$summary"
    exit 0
fi

if ! jq -e 'type == "object"' "$out_file" >/dev/null 2>&1; then
    printf 'WARN: backlog grooming preflight failed: orchestrator emitted invalid JSON\n'
    summary="$(empty_summary warn)"
    append_report "$summary"
    printf '%s\n' "$summary"
    exit 0
fi

summary="$(jq -c --arg policy "$policy" '
    {
      status: "ok",
      policy: $policy,
      promoted: (.promoted // []),
      "groom:proposed": [(.routed // [])[] | select(.action == "groom-canary" or .action == "groom:proposed") | .issue],
      held: (.held // [])
    }
' "$out_file")"
append_report "$summary"
printf '%s\n' "$summary"
