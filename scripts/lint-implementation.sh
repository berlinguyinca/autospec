#!/usr/bin/env bash
# scripts/lint-implementation.sh — implementation-quality RULE_ID detector.
#
# Usage:
#   scripts/lint-implementation.sh <PR> --issue <N>   # deterministic only, via gh pr diff
#   scripts/lint-implementation.sh --diff-file <path> # offline / pre-push
#   scripts/lint-implementation.sh --help             # print rules summary
#
# Output (default): one finding per stdout line, format:
#   RULE_ID:<path>:<line>: <one-line description>
# or for INFO (skipped):
#   INFO:RULE_ID:<path>:<line>: <opt-out: justification>
#
# Exit code = number of blocking findings (0 = pass), capped at min(N, 64).
# If findings exceed 200, exits 200 with a scope-explosion message.

set -eu

HELP_TEXT="Usage: scripts/lint-implementation.sh <PR> --issue <N>
       scripts/lint-implementation.sh --diff-file <path>
       scripts/lint-implementation.sh --help

RULE_IDs enforced (deterministic detectors):

  OUT_OF_SCOPE      Files touched are not listed in issue's ## Implementation outline.
  MISSING_TEST      Required test type from ## Tests required absent in diff.
  COMPLEXITY        Function >50 LOC, file >500 LOC, or nesting depth >4.
  SECURITY          eval(, exec(, --no-verify, git reset --hard, rm -rf /,
                    hardcoded AWS key (AKIA...), GitHub token, private key marker.
  TODO_LEFT         TODO, XXX, or FIXME found in non-test diff hunks.
  MOCK_DB           mock/stub near DB-symbol in test diff hunks.
  DOC_OUT_OF_SYNC   Public-surface change (CLI flag/env var/exported func/config key)
                    without a touched doc file (README*, AGENTS.md, docs/**, SKILL.md).

RULE_IDs checked by LLM guardian (not this script):

  HALLUCINATED_API  Symbol referenced in diff not found in repo or dependency manifests.
  DUPLICATE_CODE    New code mirrors an existing helper.
  INVENTED_CONFIG   Flag/env/key in diff not in issue body or referenced spec.

Exit code = number of blocking findings (capped at 64). Exit 0 means pass.
Exit 200 means too many findings (scope explosion)."

# ── argument parsing ──────────────────────────────────────────────────────────

PR_NUMBER=""
ISSUE_NUMBER=""
DIFF_FILE=""

while [ $# -gt 0 ]; do
    case "$1" in
        --help|-h)
            printf '%s\n' "$HELP_TEXT"
            exit 0
            ;;
        --issue)
            if [ $# -lt 2 ]; then
                printf 'lint-implementation.sh: --issue requires an argument\n' >&2
                exit 1
            fi
            ISSUE_NUMBER="$2"
            shift 2
            ;;
        --diff-file)
            if [ $# -lt 2 ]; then
                printf 'lint-implementation.sh: --diff-file requires an argument\n' >&2
                exit 1
            fi
            DIFF_FILE="$2"
            shift 2
            ;;
        -*)
            printf 'lint-implementation.sh: unknown option: %s\n' "$1" >&2
            exit 1
            ;;
        *)
            if [ -z "$PR_NUMBER" ]; then
                PR_NUMBER="$1"
            else
                printf 'lint-implementation.sh: unexpected argument: %s\n' "$1" >&2
                exit 1
            fi
            shift
            ;;
    esac
done

# Validate: must have PR_NUMBER XOR DIFF_FILE
if [ -n "$PR_NUMBER" ] && [ -n "$DIFF_FILE" ]; then
    printf 'lint-implementation.sh: --diff-file and <PR> are mutually exclusive\n' >&2
    exit 1
fi

if [ -z "$PR_NUMBER" ] && [ -z "$DIFF_FILE" ]; then
    printf 'lint-implementation.sh: must supply <PR> or --diff-file <path>\n' >&2
    printf '%s\n' "$HELP_TEXT" >&2
    exit 1
fi

if [ -n "$DIFF_FILE" ] && [ ! -f "$DIFF_FILE" ]; then
    printf 'lint-implementation.sh: diff file not found: %s\n' "$DIFF_FILE" >&2
    exit 1
fi

# ── findings accumulator ──────────────────────────────────────────────────────

FINDINGS_COUNT=0
FINDINGS_HARD_CAP=200
FINDINGS_EXIT_CAP=64

# emit_finding RULE_ID PATH LINE DESC
# Writes "RULE_ID:<path>:<line>: <desc>" to stdout and increments counter.
emit_finding() {
    local rule_id="$1"
    local path="$2"
    local line="$3"
    local desc="$4"
    printf '%s:%s:%s: %s\n' "$rule_id" "$path" "$line" "$desc"
    FINDINGS_COUNT=$((FINDINGS_COUNT + 1))
    if [ "$FINDINGS_COUNT" -ge "$FINDINGS_HARD_CAP" ]; then
        printf 'OUT_OF_SCOPE:-:-: too many findings — likely scope explosion\n'
        exit 200
    fi
}

# emit_info RULE_ID PATH LINE DESC (skip-directive honored)
emit_info() {
    local rule_id="$1"
    local path="$2"
    local line="$3"
    local desc="$4"
    printf 'INFO:%s:%s:%s: %s\n' "$rule_id" "$path" "$line" "$desc"
    # INFO lines do NOT increment blocking count
}

# ── skip-directive parsing ─────────────────────────────────────────────────────

# SKIPPED_RULES is a space-separated list of RULE_IDs that are opted out.
SKIPPED_RULES=""

# parse_skip_directives <issue-body-file>
# Reads "Guardian: skip-RULE_ID # justification" lines per §3.3 grammar.
parse_skip_directives() {
    local body_file="$1"
    if [ ! -f "$body_file" ]; then
        return
    fi
    # Grammar: ^Guardian:\s+(skip-[A-Z_]+(,\s*skip-[A-Z_]+)*)\s+#\s+\S.+$
    while IFS= read -r line; do
        if printf '%s' "$line" | grep -qE '^Guardian:[[:space:]]+(skip-[A-Z_]+(,[[:space:]]*skip-[A-Z_]+)*)[[:space:]]+#[[:space:]]+[^[:space:]].+$'; then
            # Extract the skip-X tokens
            local tokens
            tokens="$(printf '%s' "$line" | grep -oE 'skip-[A-Z_]+' | sed 's/skip-//')"
            for tok in $tokens; do
                SKIPPED_RULES="$SKIPPED_RULES $tok"
            done
        fi
    done < "$body_file"
}

# is_skipped RULE_ID — returns 0 if rule is in SKIPPED_RULES
is_skipped() {
    local rule="$1"
    case " $SKIPPED_RULES " in
        *" $rule "*) return 0 ;;
        *) return 1 ;;
    esac
}

# ── diff acquisition ──────────────────────────────────────────────────────────

TMP_DIFF="$(mktemp -t lint-impl-diff.XXXXXX)"
TMP_ISSUE="$(mktemp -t lint-impl-issue.XXXXXX)"
trap 'rm -f "$TMP_DIFF" "$TMP_ISSUE"' EXIT INT TERM

if [ -n "$DIFF_FILE" ]; then
    cp "$DIFF_FILE" "$TMP_DIFF"
else
    # Fetch diff from GitHub
    gh pr diff "$PR_NUMBER" > "$TMP_DIFF" 2>/dev/null || {
        printf 'ERROR: failed to fetch diff for PR %s\n' "$PR_NUMBER" >&2
        exit 1
    }
    # Fetch issue body for skip-directive parsing
    if [ -n "$ISSUE_NUMBER" ]; then
        gh issue view "$ISSUE_NUMBER" --json body --jq '.body' > "$TMP_ISSUE" 2>/dev/null || true
        parse_skip_directives "$TMP_ISSUE"
    fi
fi

# ── per-RULE_ID emit cap tracking ─────────────────────────────────────────────

declare -A RULE_EMIT_COUNT 2>/dev/null || {
    # Fallback for shells without associative arrays (use temp files)
    :
}
RULE_EMIT_CAP=10

# emit_capped RULE_ID PATH LINE DESC
# Honors per-RULE_ID cap of 10 lines; collapses extras to "+ N more (truncated)"
declare -A RULE_EMIT_COUNT 2>/dev/null || true
emit_capped() {
    local rule_id="$1"
    local path="$2"
    local line="$3"
    local desc="$4"

    # Get current count for this rule
    local cur=0
    # Use eval for portability across bash versions
    eval "cur=\${RULE_EMIT_COUNT_${rule_id}:-0}"
    cur=$((cur + 1))
    eval "RULE_EMIT_COUNT_${rule_id}=${cur}"

    if is_skipped "$rule_id"; then
        emit_info "$rule_id" "$path" "$line" "$desc"
        return
    fi

    if [ "$cur" -le "$RULE_EMIT_CAP" ]; then
        emit_finding "$rule_id" "$path" "$line" "$desc"
    elif [ "$cur" -eq $((RULE_EMIT_CAP + 1)) ]; then
        # Emit the truncation notice (counts as one more finding)
        emit_finding "$rule_id" "$path" "$line" "+ more (truncated)"
    fi
    # Beyond cap+1: silently drop
}

# ── detector stubs ────────────────────────────────────────────────────────────
# Each detector is a function that reads TMP_DIFF (and TMP_ISSUE) and calls
# emit_capped for each finding. Bodies are wired in issue #206.

detect_out_of_scope() {
    return 0
}

detect_missing_test() {
    return 0
}

detect_complexity() {
    return 0
}

detect_security() {
    return 0
}

detect_todo_left() {
    return 0
}

detect_mock_db() {
    return 0
}

detect_doc_out_of_sync() {
    return 0
}

# ── main ──────────────────────────────────────────────────────────────────────

detect_out_of_scope
detect_missing_test
detect_complexity
detect_security
detect_todo_left
detect_mock_db
detect_doc_out_of_sync

# Exit with min(FINDINGS_COUNT, FINDINGS_EXIT_CAP)
if [ "$FINDINGS_COUNT" -eq 0 ]; then
    exit 0
elif [ "$FINDINGS_COUNT" -gt "$FINDINGS_EXIT_CAP" ]; then
    exit "$FINDINGS_EXIT_CAP"
else
    exit "$FINDINGS_COUNT"
fi
