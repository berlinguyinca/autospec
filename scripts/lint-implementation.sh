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

RULE_EMIT_CAP=10

# emit_capped RULE_ID PATH LINE DESC
# Honors per-RULE_ID cap of 10 lines; collapses extras to "+ N more (truncated)"
emit_capped() {
    local rule_id="$1"
    local path="$2"
    local line="$3"
    local desc="$4"

    # Get current count for this rule using eval for portability
    local cur=0
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

# ── diff parsing helpers ──────────────────────────────────────────────────────

# get_diff_files — print each path touched in the diff (one per line)
get_diff_files() {
    grep -E '^diff --git ' "$TMP_DIFF" | sed 's|^diff --git a/[^ ]* b/||' || true
}

# get_added_lines_for_file FILE — print added lines (without leading +) for a file
# Scans the diff hunk by hunk, limiting to the given file section.
get_added_lines_for_file() {
    local target="$1"
    awk -v tgt="$target" '
        /^diff --git / {
            in_file = ($0 ~ " b/" tgt "$")
            next
        }
        in_file && /^\+\+\+ / { next }
        in_file && /^--- / { next }
        in_file && /^\+/ { print substr($0,2) }
    ' "$TMP_DIFF"
}

# get_added_lines_with_lineno FILE — print "LINENO:LINE" for each added line
get_added_lines_with_lineno() {
    local target="$1"
    awk -v tgt="$target" '
        /^diff --git / {
            in_file = ($0 ~ " b/" tgt "$")
            new_line = 0
            next
        }
        in_file && /^@@ / {
            # Parse @@ -old +new,count @@ — extract new-file start line
            # Portable: use sub() to strip up to + then grab the number
            hdr = $0
            sub(/.*\+/, "", hdr)
            sub(/[^0-9].*/, "", hdr)
            new_line = hdr + 0
            next
        }
        in_file && /^\+\+\+ / { next }
        in_file && /^--- / { next }
        in_file && /^ / { new_line++; next }
        in_file && /^\+/ {
            print new_line ":" substr($0,2)
            new_line++
        }
        in_file && /^-/ { next }
    ' "$TMP_DIFF"
}

# is_test_file PATH — returns 0 if path is under tests/
is_test_file() {
    case "$1" in
        tests/*) return 0 ;;
        *) return 1 ;;
    esac
}

# is_doc_file PATH — returns 0 if path is a doc file
is_doc_file() {
    case "$1" in
        README*|AGENTS.md|docs/*|*/SKILL.md|SKILL.md) return 0 ;;
        *) return 1 ;;
    esac
}

# ── §3.1 OUT_OF_SCOPE detector ────────────────────────────────────────────────
# Files touched ∉ issue body ## Implementation outline paths.
# Only active when ISSUE_NUMBER is set (needs issue body).

detect_out_of_scope() {
    # Requires issue body; skip if not available
    if [ ! -s "$TMP_ISSUE" ]; then
        return 0
    fi

    # Extract ## Implementation outline section from issue body
    local outline
    outline="$(awk '
        /^## Implementation (outline|scope)/ { in_section=1; next }
        in_section && /^## / { exit }
        in_section { print }
    ' "$TMP_ISSUE")"

    if [ -z "$outline" ]; then
        return 0
    fi

    # Extract all path-shaped tokens from the outline (backtick-quoted or bare path-like)
    local allowed_paths
    allowed_paths="$(printf '%s\n' "$outline" | grep -oE '`[^`]+`' | tr -d '`' | grep -E '[/.]' || true)"
    allowed_paths="$allowed_paths
$(printf '%s\n' "$outline" | grep -oE '[A-Za-z0-9_./\-]+\.[A-Za-z0-9]+' || true)"

    if [ -z "$(printf '%s' "$allowed_paths" | tr -d '[:space:]')" ]; then
        return 0
    fi

    # For each file in the diff, check if it matches any allowed path
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        local matched=0
        while IFS= read -r allowed; do
            [ -z "$allowed" ] && continue
            # Match: diff_file ends with allowed, or allowed is a prefix/substring
            case "$diff_file" in
                *"$allowed"*) matched=1; break ;;
            esac
            # Also check if allowed contains diff_file base
            local base
            base="$(basename "$diff_file")"
            case "$allowed" in
                *"$base"*) matched=1; break ;;
            esac
        done <<EOF
$allowed_paths
EOF
        if [ "$matched" -eq 0 ]; then
            emit_capped "OUT_OF_SCOPE" "$diff_file" "-" "file not listed in ## Implementation outline"
        fi
    done <<EOF
$(get_diff_files)
EOF
}

# ── §3.1 MISSING_TEST detector ────────────────────────────────────────────────
# Required test tier from ## Tests required not present in diff.

detect_missing_test() {
    if [ ! -s "$TMP_ISSUE" ]; then
        return 0
    fi

    # Extract ## Tests required section
    local tests_section
    tests_section="$(awk '
        /^## Tests required/ { in_section=1; next }
        in_section && /^## / { exit }
        in_section { print }
    ' "$TMP_ISSUE" | tr '[:upper:]' '[:lower:]')"

    if [ -z "$tests_section" ]; then
        return 0
    fi

    # Get list of test paths in the diff
    local diff_test_paths
    diff_test_paths="$(get_diff_files | grep '^tests/' || true)"

    # Check each tier mentioned in the tests section
    for tier in unit integration smoke e2e; do
        if printf '%s' "$tests_section" | grep -qiE "\b${tier}\b"; then
            if ! printf '%s\n' "$diff_test_paths" | grep -q "^tests/${tier}/"; then
                emit_capped "MISSING_TEST" "tests/${tier}/" "-" "required test tier '${tier}' not present in diff"
            fi
        fi
    done
}

# ── §3.1 COMPLEXITY detector ──────────────────────────────────────────────────
# Function >50 LOC, file >500 LOC, nesting >4.

detect_complexity() {
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue

        # Skip non-code files (docs, fixtures, etc.)
        case "$diff_file" in
            *.md|*.txt|*.json|*.yaml|*.yml|*.diff) continue ;;
        esac

        # Count total added lines for this file (proxy for file LOC in diff)
        local added_count
        added_count="$(get_added_lines_for_file "$diff_file" | wc -l | tr -d ' ')"

        # File LOC check: if entire new file and >500 added lines
        if [ "$added_count" -gt 500 ]; then
            emit_capped "COMPLEXITY" "$diff_file" "-" "file adds ${added_count} lines (threshold: 500)"
        fi

        # Function LOC check: scan for function definitions, count body lines
        # Strategy: track function start/end in added lines
        local in_func=0
        local func_start=0
        local func_name=""
        local func_loc=0
        local line_no=0

        while IFS=: read -r lineno content; do
            line_no="$lineno"

            # Detect function start (bash-style: name() { or function name {)
            if printf '%s' "$content" | grep -qE '^[[:space:]]*(function[[:space:]]+)?[A-Za-z_][A-Za-z0-9_]*\(\)[[:space:]]*\{?[[:space:]]*$'; then
                if [ "$in_func" -eq 1 ] && [ "$func_loc" -gt 50 ]; then
                    emit_capped "COMPLEXITY" "$diff_file" "$func_start" "function '${func_name}' is ${func_loc} LOC (threshold: 50)"
                fi
                in_func=1
                func_start="$lineno"
                func_name="$(printf '%s' "$content" | grep -oE '[A-Za-z_][A-Za-z0-9_]*' | head -1)"
                func_loc=0
                continue
            fi

            if [ "$in_func" -eq 1 ]; then
                # Detect closing brace at column 0 (end of function)
                if printf '%s' "$content" | grep -qE '^}[[:space:]]*$'; then
                    func_loc=$((func_loc + 1))
                    if [ "$func_loc" -gt 50 ]; then
                        emit_capped "COMPLEXITY" "$diff_file" "$func_start" "function '${func_name}' is ${func_loc} LOC (threshold: 50)"
                    fi
                    in_func=0
                    func_loc=0
                else
                    func_loc=$((func_loc + 1))
                fi
            fi

            # Nesting depth check: count leading spaces / 4 as proxy for depth
            local leading_spaces
            leading_spaces="$(printf '%s' "$content" | sed 's/[^ ].*//' | wc -c | tr -d ' ')"
            # leading_spaces includes the newline from wc -c, adjust
            leading_spaces=$((leading_spaces - 1))
            local depth=$((leading_spaces / 4))
            if [ "$depth" -gt 4 ]; then
                emit_capped "COMPLEXITY" "$diff_file" "$lineno" "nesting depth ~${depth} (threshold: 4) at line ${lineno}"
            fi
        done <<EOF
$(get_added_lines_with_lineno "$diff_file")
EOF

        # Flush last open function
        if [ "$in_func" -eq 1 ] && [ "$func_loc" -gt 50 ]; then
            emit_capped "COMPLEXITY" "$diff_file" "$func_start" "function '${func_name}' is ${func_loc} LOC (threshold: 50)"
        fi

    done <<EOF
$(get_diff_files)
EOF
}

# ── §3.1 SECURITY detector ────────────────────────────────────────────────────
# Dangerous patterns in any diff hunk.

# scan_security_pattern PAT DESC — scan TMP_DIFF added lines for pattern
scan_security_pattern() {
    local pat="$1"
    local desc="$2"
    local current_file="-"
    local current_line=0
    while IFS= read -r raw_line; do
        if printf '%s' "$raw_line" | grep -qE '^diff --git '; then
            current_file="$(printf '%s' "$raw_line" | sed 's|^diff --git a/[^ ]* b/||')"
            current_line=0
            continue
        fi
        if printf '%s' "$raw_line" | grep -qE '^@@ '; then
            local hdr="$raw_line"
            hdr="$(printf '%s' "$raw_line" | grep -oE '\+[0-9]+' | head -1 | tr -d '+')"
            current_line="${hdr:-0}"
            continue
        fi
        if printf '%s' "$raw_line" | grep -qE '^\+\+\+|^---'; then
            continue
        fi
        if printf '%s' "$raw_line" | grep -qE '^ '; then
            current_line=$((current_line + 1))
            continue
        fi
        if printf '%s' "$raw_line" | grep -qE '^\+'; then
            current_line=$((current_line + 1))
            local content="${raw_line#\+}"
            if printf '%s' "$content" | grep -qE "$pat"; then
                emit_capped "SECURITY" "$current_file" "$current_line" "$desc"
            fi
        fi
    done < "$TMP_DIFF"
}

detect_security() {
    scan_security_pattern 'eval\(' "eval() usage — potential code injection"
    scan_security_pattern 'exec\(' "exec() usage — potential code injection"
    scan_security_pattern '\-\-no\-verify' "--no-verify flag — bypasses git hooks"
    scan_security_pattern 'git reset --hard' "git reset --hard — destructive operation"
    scan_security_pattern 'rm -rf /' "rm -rf / — dangerous recursive delete"
    scan_security_pattern 'AKIA[0-9A-Z]{16}' "hardcoded AWS access key (AKIA...)"
    scan_security_pattern 'gh[pousr]_[A-Za-z0-9]{36,}' "hardcoded GitHub token"
    scan_security_pattern '\-\-\-\-\-BEGIN [A-Z ]*PRIVATE KEY\-\-\-\-\-' "private key material in committed file"
}

# ── §3.1 TODO_LEFT detector ───────────────────────────────────────────────────
# \b(TODO|XXX|FIXME)\b in non-test added diff hunks.

detect_todo_left() {
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        is_test_file "$diff_file" && continue

        while IFS=: read -r lineno content; do
            if printf '%s' "$content" | grep -qE '\b(TODO|XXX|FIXME)\b'; then
                local match
                match="$(printf '%s' "$content" | grep -oE '\b(TODO|XXX|FIXME)\b' | head -1)"
                emit_capped "TODO_LEFT" "$diff_file" "$lineno" "${match} found in non-test source"
            fi
        done <<EOF
$(get_added_lines_with_lineno "$diff_file")
EOF
    done <<EOF
$(get_diff_files)
EOF
}

# ── §3.1 MOCK_DB detector ─────────────────────────────────────────────────────
# \b(mock|stub)\b near DB-symbol heuristics in test diff hunks.

detect_mock_db() {
    local db_pattern='(db\.|database|DataSource|pg|mysql|sqlite)'
    local mock_pattern='\b(mock|stub)\b'

    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        is_test_file "$diff_file" || continue

        while IFS=: read -r lineno content; do
            if printf '%s' "$content" | grep -qiE "$mock_pattern"; then
                if printf '%s' "$content" | grep -qiE "$db_pattern"; then
                    emit_capped "MOCK_DB" "$diff_file" "$lineno" "mock/stub of DB symbol detected in test"
                fi
            fi
        done <<EOF
$(get_added_lines_with_lineno "$diff_file")
EOF
    done <<EOF
$(get_diff_files)
EOF
}

# ── §3.1 DOC_OUT_OF_SYNC detector (det half) ─────────────────────────────────
# Any change to public surface (CLI flag/env var/exported func/config key)
# without a touched doc file.

detect_doc_out_of_sync() {
    local diff_files
    diff_files="$(get_diff_files)"

    # Check if any doc file was touched
    local doc_touched=0
    while IFS= read -r f; do
        [ -z "$f" ] && continue
        if is_doc_file "$f"; then
            doc_touched=1
            break
        fi
    done <<EOF
$diff_files
EOF

    if [ "$doc_touched" -eq 1 ]; then
        return 0
    fi

    # Look for public-surface changes in non-test, non-doc source files
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        is_test_file "$diff_file" && continue
        is_doc_file "$diff_file" && continue

        # Skip binary-ish and fixture files
        case "$diff_file" in
            *.diff|*.png|*.jpg|*.gif) continue ;;
        esac

        while IFS=: read -r lineno content; do
            # CLI flag pattern: --flag-name with at least 3 chars (avoid -f style short flags in comments)
            if printf '%s' "$content" | grep -qE '(^|[[:space:]])(--[a-z][a-z0-9-]{2,})[[:space:]=]'; then
                emit_capped "DOC_OUT_OF_SYNC" "$diff_file" "$lineno" "CLI long-flag introduced without touching a doc file"
                break
            fi
            # Env var export pattern: export FOO_BAR= or FOO_BAR= at start of line (require underscore or 3+ caps)
            if printf '%s' "$content" | grep -qE '^(export[[:space:]]+)?[A-Z][A-Z0-9]*_[A-Z0-9_]+='; then
                emit_capped "DOC_OUT_OF_SYNC" "$diff_file" "$lineno" "env var introduced without touching a doc file"
                break
            fi
        done <<EOF
$(get_added_lines_with_lineno "$diff_file")
EOF
    done <<EOF
$diff_files
EOF
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
