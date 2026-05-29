#!/usr/bin/env bash
# scripts/autospec-write-run-summary.sh
#
# Phase 6 canonical run-summary writer. Assembles the run state into
# .autospec/run-summary.md per the schema documented in
# skills/autospec/SKILL.md §Phase 6, and writes it atomically via .tmp + mv.
#
# Schema:
#   # autospec run summary — <ISO-8601 UTC timestamp>
#
#   - HEAD SHA: <sha>
#   - Branch: <branch>
#   - Total PRs merged: N
#   - Issues closed: M
#   - Elapsed: HH:MM
#
#   ## PRs merged
#   - #1234 fix: ...
#
#   ## Issues closed
#   - #1230 ...
#
#   ## Failures
#   (empty or list)
#
#   ## Next steps
#   <content from --next-steps-file, or "- (none — converged)" fallback>
#
# Inputs are passed as flags. All flags optional except --output; missing
# flags resolve to safe defaults so the harvest contract still produces a
# schema-valid file (## Next steps section is always present).
#
# Exit codes: 0 success; 1 invocation error; 2 write failure.

set -eu

usage() {
    cat <<'EOF'
Usage: autospec-write-run-summary.sh [options]

Options:
  --repo <slug>             owner/name repo slug (defaults: "")
  --prs <file|csv>          file with one PR per line OR comma-separated list
  --issues <file|csv>       file with one issue per line OR comma-separated list
  --failures <file|csv>     file with failures OR comma-separated list
  --elapsed <HH:MM>         elapsed wall-clock time (defaults: "")
  --next-steps-file <file>  file with markdown for the ## Next steps section
                            (defaults to "- (none — converged)" when empty/missing)
  --output <path>           output markdown path (REQUIRED)
  --sha <sha>               HEAD SHA override (defaults: git rev-parse HEAD)
  --branch <name>           branch override (defaults: git symbolic-ref HEAD)
  -h, --help                show this help and exit
EOF
}

REPO=""
PRS_INPUT=""
ISSUES_INPUT=""
FAILURES_INPUT=""
ELAPSED=""
NEXT_STEPS_FILE=""
OUTPUT=""
SHA_OVERRIDE=""
BRANCH_OVERRIDE=""

while [ $# -gt 0 ]; do
    case "$1" in
        --repo) REPO="${2:-}"; shift 2 ;;
        --prs) PRS_INPUT="${2:-}"; shift 2 ;;
        --issues) ISSUES_INPUT="${2:-}"; shift 2 ;;
        --failures) FAILURES_INPUT="${2:-}"; shift 2 ;;
        --elapsed) ELAPSED="${2:-}"; shift 2 ;;
        --next-steps-file) NEXT_STEPS_FILE="${2:-}"; shift 2 ;;
        --output) OUTPUT="${2:-}"; shift 2 ;;
        --sha) SHA_OVERRIDE="${2:-}"; shift 2 ;;
        --branch) BRANCH_OVERRIDE="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown flag: $1" >&2; usage >&2; exit 1 ;;
    esac
done

if [ -z "$OUTPUT" ]; then
    echo "ERROR: --output is required" >&2
    usage >&2
    exit 1
fi

# Resolve git state with defaults (so the helper works in test fixtures
# outside a real repo).
if [ -n "$SHA_OVERRIDE" ]; then
    SHA="$SHA_OVERRIDE"
elif command -v git >/dev/null 2>&1 && git rev-parse HEAD >/dev/null 2>&1; then
    SHA=$(git rev-parse HEAD 2>/dev/null || echo "unknown")
else
    SHA="unknown"
fi

if [ -n "$BRANCH_OVERRIDE" ]; then
    BRANCH="$BRANCH_OVERRIDE"
elif command -v git >/dev/null 2>&1 && git symbolic-ref --short HEAD >/dev/null 2>&1; then
    BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || echo "unknown")
else
    BRANCH="unknown"
fi

TIMESTAMP=$(date -u +'%Y-%m-%dT%H:%M:%SZ')

# Render an input list as one-bullet-per-line markdown. Accepts either a path
# to a file (one entry per line) or a comma-separated string.
render_list() {
    raw="$1"
    if [ -z "$raw" ]; then
        return 0
    fi
    if [ -f "$raw" ]; then
        # File: emit lines verbatim, prefixing "- " if missing.
        while IFS= read -r line || [ -n "$line" ]; do
            [ -z "$line" ] && continue
            case "$line" in
                -\ *) printf '%s\n' "$line" ;;
                *) printf -- '- %s\n' "$line" ;;
            esac
        done < "$raw"
    else
        # CSV: split on commas, one bullet each.
        # shellcheck disable=SC2001
        echo "$raw" | tr ',' '\n' | while IFS= read -r item; do
            item=$(printf '%s' "$item" | sed 's/^ *//;s/ *$//')
            [ -z "$item" ] && continue
            case "$item" in
                -\ *) printf '%s\n' "$item" ;;
                *) printf -- '- %s\n' "$item" ;;
            esac
        done
    fi
}

count_list() {
    raw="$1"
    if [ -z "$raw" ]; then
        echo 0
        return
    fi
    if [ -f "$raw" ]; then
        grep -c '[^[:space:]]' "$raw" 2>/dev/null || echo 0
    else
        echo "$raw" | tr ',' '\n' | grep -c '[^[:space:]]' || echo 0
    fi
}

PRS_RENDERED=$(render_list "$PRS_INPUT")
ISSUES_RENDERED=$(render_list "$ISSUES_INPUT")
FAILURES_RENDERED=$(render_list "$FAILURES_INPUT")
PR_COUNT=$(count_list "$PRS_INPUT")
ISSUE_COUNT=$(count_list "$ISSUES_INPUT")

# Resolve ## Next steps content. The section is REQUIRED — fall back to
# `- (none — converged)` so the harvest contract sees convergence_clean.
if [ -n "$NEXT_STEPS_FILE" ] && [ -f "$NEXT_STEPS_FILE" ] \
    && grep -q '[^[:space:]]' "$NEXT_STEPS_FILE" 2>/dev/null; then
    NEXT_STEPS_BODY=$(cat "$NEXT_STEPS_FILE")
else
    NEXT_STEPS_BODY="- (none — converged)"
fi

# Ensure the output directory exists.
OUT_DIR=$(dirname "$OUTPUT")
mkdir -p "$OUT_DIR" || { echo "ERROR: cannot create $OUT_DIR" >&2; exit 2; }

TMP="${OUTPUT}.tmp.$$"

{
    printf '# autospec run summary — %s\n\n' "$TIMESTAMP"
    printf -- '- HEAD SHA: %s\n' "$SHA"
    printf -- '- Branch: %s\n' "$BRANCH"
    if [ -n "$REPO" ]; then
        printf -- '- Repo: %s\n' "$REPO"
    fi
    printf -- '- Total PRs merged: %s\n' "$PR_COUNT"
    printf -- '- Issues closed: %s\n' "$ISSUE_COUNT"
    printf -- '- Elapsed: %s\n' "${ELAPSED:-unknown}"
    printf '\n## PRs merged\n\n'
    if [ -n "$PRS_RENDERED" ]; then
        printf '%s\n' "$PRS_RENDERED"
    else
        printf '(none)\n'
    fi
    printf '\n## Issues closed\n\n'
    if [ -n "$ISSUES_RENDERED" ]; then
        printf '%s\n' "$ISSUES_RENDERED"
    else
        printf '(none)\n'
    fi
    printf '\n## Failures\n\n'
    if [ -n "$FAILURES_RENDERED" ]; then
        printf '%s\n' "$FAILURES_RENDERED"
    else
        printf '(none)\n'
    fi
    printf '\n## Next steps\n\n'
    printf '%s\n' "$NEXT_STEPS_BODY"
} > "$TMP" || { rm -f "$TMP"; echo "ERROR: write failed" >&2; exit 2; }

mv -f "$TMP" "$OUTPUT" || { rm -f "$TMP"; echo "ERROR: mv failed" >&2; exit 2; }

exit 0
