#!/usr/bin/env bash
# bootstrap-labels.sh — create the full e2e:* label set idempotently.
#
# Usage:
#   bootstrap-labels.sh [--repo <owner/repo>] [--dry-run]
#
# Creates 16 labels via `gh label create --force` (idempotent).
# --dry-run: print the labels that would be created without touching GitHub.
#
# Exit codes:
#   0 = success (or dry-run complete)
#   1 = error (gh CLI failure, missing auth)

set -eu

REPO_FLAG=""
DRY_RUN=false

while [ $# -gt 0 ]; do
    case "$1" in
        --repo)    REPO_FLAG="--repo ${2:-}"; shift 2 ;;
        --dry-run) DRY_RUN=true; shift ;;
        *) printf 'bootstrap-labels: unknown flag: %s\n' "$1" >&2; exit 1 ;;
    esac
done

# ── Label definitions ──────────────────────────────────────────────────────────
# Format: "name|color|description" — pipe delimiter avoids conflict with e2e:* colons

LABELS=(
    "e2e:passed|0e8a16|autospec-test Stage 1+2 passed"
    "e2e:healed|0e8a16|autospec-test self-heal loop fixed the gate"
    "e2e:blocked|d73a4a|autospec-test gate blocked — needs action"
    "e2e:refused|b60205|autospec-test refused to run (contract error)"
    "e2e:contract-error|e4e669|autospec-test contract invalid or tool missing"
    "e2e:assertion-loosening|fbca04|autospec-test detected assertion loosening"
    "e2e:unjustified-shift|d93f0b|autospec-test assertion shift without JUSTIFICATION"
    "e2e:stuck-error|d73a4a|autospec-test same error 3 consecutive iterations"
    "e2e:no-action|cccccc|autospec-test loop classifier produced empty action 2x"
    "e2e:scoped-prod|1d76db|autospec-test ran in scoped-production mode"
    "e2e:scoped-prod-quarantined|e4e669|autospec-test Mode II auto-disabled (2 violations)"
    "e2e:scope-violation|d73a4a|autospec-test test accessed out-of-scope data"
    "e2e:restored|0e8a16|autospec-test restore succeeded after scope violation"
    "e2e:restore-failed|b60205|autospec-test restore FAILED after scope violation"
    "CRITICAL|b60205|autospec-test restore failed — operator intervention required"
    "needs-human-review|fbca04|PR needs human review before merge"
)

if [ "$DRY_RUN" = "true" ]; then
    printf 'bootstrap-labels: dry-run — would create %d labels:\n' "${#LABELS[@]}"
    for entry in "${LABELS[@]}"; do
        IFS='|' read -r name color desc <<< "$entry"
        printf '  %-40s  #%s  %s\n' "$name" "$color" "$desc"
    done
    exit 0
fi

if ! command -v gh >/dev/null 2>&1; then
    printf 'bootstrap-labels: gh CLI not found\n' >&2
    exit 1
fi

ERRORS=0
for entry in "${LABELS[@]}"; do
    IFS='|' read -r name color desc <<< "$entry"
    if ! gh label create "$name" \
        --color "$color" \
        --description "$desc" \
        --force \
        $REPO_FLAG 2>/dev/null; then
        printf 'bootstrap-labels: WARN: failed to create label: %s\n' "$name" >&2
        ERRORS=$((ERRORS + 1))
    else
        printf 'bootstrap-labels: created/updated: %s\n' "$name"
    fi
done

if [ "$ERRORS" -gt 0 ]; then
    printf 'bootstrap-labels: %d label(s) failed\n' "$ERRORS" >&2
    exit 1
fi

printf 'bootstrap-labels: all %d labels created/updated successfully\n' "${#LABELS[@]}"
