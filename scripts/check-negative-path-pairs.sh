#!/usr/bin/env bash
# scripts/check-negative-path-pairs.sh -- negative-path pair heuristic (spec s6).
#
# Usage:
#   bash scripts/check-negative-path-pairs.sh <test-file> [<test-file2> ...]
#   bash scripts/check-negative-path-pairs.sh --diff-file <path>
#   bash scripts/check-negative-path-pairs.sh --staged
#
# For each test name matching a "positive" pattern (should succeed / returns / works),
# look for a sibling test name matching a "negative" pattern (should fail / rejects / throws).
# Missing pair emits WARN finding (not block). Exit 0 always.
#
# Output format:
#   WARN:MISSING_NEGATIVE_PATH:<file>:<line>: test '<name>' has no negative-path sibling
#
# Overlay: loads per-language patterns from <repo-root>/negative-path-patterns.yml if present.

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Default patterns per spec s6 — use word boundaries to avoid subword matches (e.g. invalid matching valid)
POSITIVE_PATTERN='(should|when).*\b(success|valid|pass|works|returns|succeed)\b'
NEGATIVE_PATTERN='(should|when).*\b(fail|invalid|reject|throws|errors|error)\b'

# Load overlay from negative-path-patterns.yml
OVERLAY_FILE="$REPO_ROOT/negative-path-patterns.yml"
if [ -f "$OVERLAY_FILE" ]; then
    _pos="$(grep -E '^positive_pattern:' "$OVERLAY_FILE" 2>/dev/null \
        | head -1 | sed 's/positive_pattern:[[:space:]]*//' | tr -d '"' || true)"
    _neg="$(grep -E '^negative_pattern:' "$OVERLAY_FILE" 2>/dev/null \
        | head -1 | sed 's/negative_pattern:[[:space:]]*//' | tr -d '"' || true)"
    [ -n "$_pos" ] && POSITIVE_PATTERN="$_pos"
    [ -n "$_neg" ] && NEGATIVE_PATTERN="$_neg"
fi

# Argument parsing
FILES_TO_CHECK=""
DIFF_FILE=""
STAGED=0

while [ $# -gt 0 ]; do
    _a="$1"
    case "$_a" in
        --diff-file)
            DIFF_FILE="${2:-}"
            shift 2 ;;
        --staged)
            STAGED=1
            shift ;;
        --help)
            grep '^#' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        -*)
            printf 'check-negative-path-pairs.sh: unknown option: %s\n' "$_a" >&2
            exit 1 ;;
        *)
            FILES_TO_CHECK="${FILES_TO_CHECK:+$FILES_TO_CHECK }$_a"
            shift ;;
    esac
done

# Extract test name from a single line of text
# Writes the test name to stdout, or nothing if not recognized
_extract_test_name() {
    local line="$1"
    # bats: @test "name" {
    if printf '%s' "$line" | grep -qE '@test[[:space:]]+"'; then
        printf '%s' "$line" | grep -oE '"[^"]*"' | head -1 | tr -d '"'
        return
    fi
    # JS/TS: it("name" or test("name"
    if printf '%s' "$line" | grep -qE '(^|[[:space:]])(it|test)[[:space:]]*\('; then
        printf '%s' "$line" | grep -oE '[(][^)]*[)]' | head -1 \
            | sed "s/^[(]//" | sed "s/[)][^)]*$//" \
            | tr -d '"' | tr -d "'"
        return
    fi
    # Python: def test_name(
    if printf '%s' "$line" | grep -qE 'def[[:space:]]+test_[A-Za-z0-9_]+'; then
        printf '%s' "$line" | grep -oE 'test_[A-Za-z0-9_]+' | head -1 | tr '_' ' '
        return
    fi
    # Go: func TestName(
    if printf '%s' "$line" | grep -qE 'func[[:space:]]+Test[A-Z][A-Za-z0-9_]*'; then
        printf '%s' "$line" | grep -oE 'Test[A-Za-z0-9_]+' | head -1 \
            | sed 's/\([A-Z]\)/ \1/g' | tr '[:upper:]' '[:lower:]' | sed 's/^ //'
        return
    fi
}

# Classify test names from a file into pos_tmp and neg_tmp temp files
_classify_test_names() {
    local file="$1" pos_tmp="$2" neg_tmp="$3"
    local lineno=0 test_name
    while IFS= read -r line; do
        lineno=$((lineno + 1))
        test_name="$(_extract_test_name "$line")"
        [ -z "$test_name" ] && continue
        if printf '%s' "$test_name" | grep -qiE "$POSITIVE_PATTERN"; then
            printf '%d:%s\n' "$lineno" "$test_name" >> "$pos_tmp"
        elif printf '%s' "$test_name" | grep -qiE "$NEGATIVE_PATTERN"; then
            printf '%s\n' "$test_name" >> "$neg_tmp"
        fi
    done < "$file"
}

# Emit WARN for each positive test in pos_tmp that lacks a negative sibling in neg_tmp
_emit_missing_pairs() {
    local file="$1" pos_tmp="$2" neg_tmp="$3"
    local entry pos_line pos_name subject pat first_word found
    while IFS= read -r entry; do
        [ -z "$entry" ] && continue
        pos_line="$(printf '%s' "$entry" | cut -d: -f1)"
        pos_name="$(printf '%s' "$entry" | cut -d: -f2-)"
        [ -z "$pos_name" ] && continue
        subject="$(printf '%s' "$pos_name" \
            | sed -E "s/ (success|valid|pass|works|returns|succeed|should|when).*//i" \
            | sed 's/^[[:space:]]*//' | sed 's/[[:space:]]*$//')"
        found=0
        if [ -s "$neg_tmp" ]; then
            if [ -n "$subject" ]; then
                pat="$(printf '%s' "$subject" | sed 's/[[:space:]]\{1,\}/.*/g')"
                grep -qiE "$pat" "$neg_tmp" 2>/dev/null && found=1
            fi
            if [ "$found" -eq 0 ]; then
                first_word="$(printf '%s' "$subject" | cut -d' ' -f1)"
                [ -n "$first_word" ] && grep -qiF "$first_word" "$neg_tmp" 2>/dev/null && found=1
            fi
        fi
        if [ "$found" -eq 0 ]; then
            printf 'WARN:MISSING_NEGATIVE_PATH:%s:%s: test [%s] has no negative-path sibling\n' \
                "$file" "$pos_line" "$pos_name"
        fi
    done < "$pos_tmp"
}

# Analyze a single test file for missing negative-path pairs
_check_file() {
    local file="$1"
    if [ ! -f "$file" ]; then
        printf 'check-negative-path-pairs.sh: file not found: %s\n' "$file" >&2
        return
    fi
    local pos_tmp neg_tmp
    pos_tmp="$(mktemp -t negpath-pos.XXXXXX)"
    neg_tmp="$(mktemp -t negpath-neg.XXXXXX)"
    # No RETURN trap — inline cleanup (feedback_bash_return_trap_leak.md)
    _classify_test_names "$file" "$pos_tmp" "$neg_tmp"
    _emit_missing_pairs "$file" "$pos_tmp" "$neg_tmp"
    rm -f "$pos_tmp" "$neg_tmp"
}

# Extract changed test files from a diff
_get_changed_test_files_from_diff() {
    local diff_content="$1"
    printf '%s\n' "$diff_content" | grep '^+++ b/' | sed 's|^+++ b/||' \
        | grep -E '\.(bats|spec\.[jt]s|_test\.go|test_.*\.py|_spec\.rb|test\.sh)$' || true
}

# Main dispatch
if [ -n "$FILES_TO_CHECK" ]; then
    for f in $FILES_TO_CHECK; do
        _check_file "$f"
    done
elif [ -n "$DIFF_FILE" ]; then
    diff_content="$(cat "$DIFF_FILE")"
    _get_changed_test_files_from_diff "$diff_content" | while IFS= read -r f; do
        [ -z "$f" ] && continue
        [ -f "$REPO_ROOT/$f" ] && _check_file "$REPO_ROOT/$f"
    done
elif [ "$STAGED" -eq 1 ]; then
    diff_content="$(git diff --cached 2>/dev/null || true)"
    _get_changed_test_files_from_diff "$diff_content" | while IFS= read -r f; do
        [ -z "$f" ] && continue
        [ -f "$REPO_ROOT/$f" ] && _check_file "$REPO_ROOT/$f"
    done
fi

exit 0
