#!/usr/bin/env bash
# scripts/pre-commit-loop-guard.sh
# Pre-commit hook installed into the target worktree.
# Rejects edits to loop-immutable files per spec §3.
#
# Loop-immutable paths:
#   - .autospec/test.yml
#   - .autospec/.scoped-prod-acked-*.lock
#   - playwright.config.* safety-related fields (forbidden_url_patterns, mode)
#
# Install: cp pre-commit-loop-guard.sh <worktree>/.git/hooks/pre-commit
#          chmod +x <worktree>/.git/hooks/pre-commit

set -eu

VIOLATIONS=()

# Get list of staged files
STAGED=$(git diff --cached --name-only 2>/dev/null || true)

for file in $STAGED; do
    case "$file" in
        # .autospec/test.yml — fully immutable during loop
        .autospec/test.yml)
            VIOLATIONS+=("$file: loop-immutable (spec §3 — edit .autospec/test.yml outside the loop)")
            ;;

        # .autospec/.scoped-prod-acked-*.lock — immutable during loop
        .autospec/.scoped-prod-acked-*.lock)
            VIOLATIONS+=("$file: loop-immutable (spec §3 — scoped-prod ack lock must not change during loop)")
            ;;

        # playwright.config.* — check for safety-related field changes
        playwright.config.ts|playwright.config.js|playwright.config.mjs|playwright.config.cjs)
            # Check if the diff touches forbidden_url_patterns or mode fields
            DIFF=$(git diff --cached -- "$file" 2>/dev/null || true)
            if printf '%s\n' "$DIFF" | grep -qE '^\+[^+].*forbidden_url_patterns'; then
                VIOLATIONS+=("$file: loop-immutable field 'forbidden_url_patterns' must not be changed during loop (spec §3)")
            fi
            if printf '%s\n' "$DIFF" | grep -qE '^\+[^+].*\bmode\s*:'; then
                VIOLATIONS+=("$file: loop-immutable field 'mode' must not be changed during loop (spec §3)")
            fi
            ;;
    esac
done

if [ "${#VIOLATIONS[@]}" -gt 0 ]; then
    printf '\n[autospec loop-guard] COMMIT REJECTED — loop-immutable files modified:\n\n' >&2
    for v in "${VIOLATIONS[@]}"; do
        printf '  ✗ %s\n' "$v" >&2
    done
    printf '\nThese files cannot be modified by the self-heal loop (spec §3).\n' >&2
    printf 'Fix the loop logic instead of relaxing safety constraints.\n\n' >&2
    exit 1
fi

exit 0
