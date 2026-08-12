#!/usr/bin/env bash
# scripts/lint-implementation.sh — implementation-quality RULE_ID detector.
#
# Usage:
#   scripts/lint-implementation.sh <PR> --issue <N>   # deterministic only, via gh pr diff
#   scripts/lint-implementation.sh --diff-file <path> # offline / pre-push
#   scripts/lint-implementation.sh --pre-commit --staged  # staged diff (pre-commit hook mode)
#   scripts/lint-implementation.sh --staged --staged-base <commit>
#   scripts/lint-implementation.sh --directives       # reformat findings as directive lines
#   scripts/lint-implementation.sh --help             # print rules summary
#
# Output (default): one finding per stdout line, format:
#   RULE_ID:<path>:<line>: <one-line description>
# or for INFO (skipped):
#   INFO:RULE_ID:<path>:<line>: <opt-out: justification>
#
# With --directives: one imperative directive per finding:
#   Fix <RULE_ID>: <imperative action>
#
# Exit code = number of blocking findings (0 = pass), capped at min(N, 64).
# If findings exceed 200, exits 200 with a scope-explosion message.

set -eu
# Keep operational diagnostics visible when directive mode captures detector output.
exec 3>&2
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"

HELP_TEXT="Usage: scripts/lint-implementation.sh <PR> --issue <N>
       scripts/lint-implementation.sh --diff-file <path>
       scripts/lint-implementation.sh --pre-commit --staged
       scripts/lint-implementation.sh --staged --staged-base <commit>
       scripts/lint-implementation.sh [--directives]
       scripts/lint-implementation.sh --help

Flags:
  --pre-commit      Run in pre-commit mode (reads staged diff instead of PR diff)
  --staged          Alias for --pre-commit; use git diff --cached as diff source
  --staged-base     Compare the staged index to one explicit commit
  --directives      Reformat each finding as an imperative directive line:
                    \"Fix <RULE_ID>: <imperative action>\"
                    Suitable for injecting into implementer retry prompts.
  --vacuous-assertions  Run vacuous-assertion detector (bundled in --pre-commit).
                    Detects 8 patterns where tests always pass regardless of behavior.

RULE_IDs enforced (deterministic detectors):

  OUT_OF_SCOPE      Files touched are not listed in issue's ## Implementation outline.
  PR_SIZE           Patch exceeds 400 changed lines, 8 files, 3 logical units,
                    or contains binary diff evidence.
  MISSING_TEST      Required test type from ## Tests required absent in diff.
  COMPLEXITY        Function >50 LOC, file over AUTOSPEC_MAX_FILE_LOC, or nesting depth >4. Advisory (INFO) unless AUTOSPEC_COMPLEXITY_ENFORCE=1.
  SECURITY          eval(, exec(, --no-verify, git reset --hard, rm -rf /,
                    hardcoded AWS key (AKIA...), GitHub token, private key marker.
  TODO_LEFT         TODO, XXX, or FIXME found in non-test diff hunks.
  MOCK_DB           mock/stub near DB-symbol in test diff hunks.
  DOC_OUT_OF_SYNC   Public-surface change (CLI flag/env var/exported func/config key)
                    without a touched doc file (README*, AGENTS.md, docs/**, SKILL.md).
  VACUOUS_GREP_INVERSE_OR_TRUE  grep -qv ... || true — always passes; assertion is a no-op.
  VACUOUS_OR_TRUE               || true at end of any test assertion line — masks failures.
  VACUOUS_TAUTOLOGY             expect(true).toBe(true), assert(1===1), assert True, xit(...).
  VACUOUS_AC_STUB               @test ... { skip \"auto-stub\" } in tests/ac/ — auto-generated stub.
  VACUOUS_EMPTY_TEST            Empty test body: it(\"...\", () => {}) or @test with only braces.
  VACUOUS_NO_ASSERT             Loop or function test body with no assert/expect call (WARN).

  REINVENT_REPO_UTIL  Net-new function whose name duplicates an existing helper found by rg
                    across scripts/. Opt-out: # linter:allow-REINVENT_REPO_UTIL <reason>.
  NEW_DEP_UNJUSTIFIED  Dependency added to a manifest (requirements.txt, package.json,
                    go.mod, Cargo.toml, pyproject.toml, Gemfile) without a 'why:' marker
                    in the same hunk. Opt-out: # linter:allow-NEW_DEP_UNJUSTIFIED <reason>.
  NEW_ABSTRACTION_SINGLE_CALLER  New *manager*|*factory*|*adapter*|*wrapper*|*base*|*abstract*
                    file with ≤1 external call site found by rg. Opt-out:
                    # linter:allow-NEW_ABSTRACTION_SINGLE_CALLER <reason>.

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
PRE_COMMIT=0
STAGED=0
STAGED_BASE=""
DIRECTIVES=0
VACUOUS_ASSERTIONS=0
ASSERTION_DENSITY=0

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
        --pre-commit)
            PRE_COMMIT=1
            STAGED=1
            VACUOUS_ASSERTIONS=1
            ASSERTION_DENSITY=1
            shift
            ;;
        --staged)
            STAGED=1
            shift
            ;;
        --staged-base)
            if [ $# -lt 2 ] || [ -n "$STAGED_BASE" ]; then
                printf 'lint-implementation.sh: --staged-base requires one commit\n' >&2
                exit 1
            fi
            STAGED_BASE="$2"
            shift 2
            ;;
        --directives)
            DIRECTIVES=1
            shift
            ;;
        --vacuous-assertions)
            VACUOUS_ASSERTIONS=1
            shift
            ;;
        --assertion-density)
            ASSERTION_DENSITY=1
            shift
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

# Validate: must have PR_NUMBER XOR DIFF_FILE XOR --staged
if [ -n "$PR_NUMBER" ] && [ -n "$DIFF_FILE" ]; then
    printf 'lint-implementation.sh: --diff-file and <PR> are mutually exclusive\n' >&2
    exit 1
fi

if [ "$STAGED" -eq 0 ] && [ -z "$PR_NUMBER" ] && [ -z "$DIFF_FILE" ]; then
    printf 'lint-implementation.sh: must supply <PR>, --diff-file <path>, or --staged\n' >&2
    printf '%s\n' "$HELP_TEXT" >&2
    exit 1
fi

if [ -n "$STAGED_BASE" ]; then
    if [ "$STAGED" -eq 0 ]; then
        printf 'lint-implementation.sh: --staged-base requires --staged\n' >&2
        exit 1
    fi
    if ! STAGED_BASE="$(git rev-parse --verify "$STAGED_BASE^{commit}" 2>/dev/null)"; then
        printf 'lint-implementation.sh: invalid staged base commit\n' >&2
        exit 1
    fi
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

# emit_error RULE_ID PATH LINE DESC (explicit severity for hard admission gates)
emit_error() {
    local rule_id="$1" path="$2" line="$3" desc="$4"
    printf 'ERROR:%s:%s:%s: %s\n' "$rule_id" "$path" "$line" "$desc"
    FINDINGS_COUNT=$((FINDINGS_COUNT + 1))
    if [ "$FINDINGS_COUNT" -ge "$FINDINGS_HARD_CAP" ]; then
        printf 'OUT_OF_SCOPE:-:-: too many findings — likely scope explosion\n'
        exit 200
    fi
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

# ── inline escape-hatch (linter:allow-) ──────────────────────────────────────

# is_line_allowed RULE_ID FILE LINENO
# Returns 0 (allowed/suppressed) if the line at LINENO in FILE, or the
# immediately preceding line, contains a valid inline escape hatch comment:
#   # linter:allow-RULE_ID <reason>
# A bare "# linter:allow-X" without a reason is NOT honored.
is_line_allowed() {
    local rule_id="$1"
    local file="$2"
    local lineno="$3"
    [ -f "$file" ] || return 1
    local pattern="linter:allow-${rule_id}[[:space:]]+[^[:space:]]"
    # Check the line itself and the line immediately above
    local check_line prev_line=""
    check_line="$(sed -n "${lineno}p" "$file" 2>/dev/null || true)"
    if [ "$lineno" -gt 1 ]; then
        prev_line="$(sed -n "$((lineno - 1))p" "$file" 2>/dev/null || true)"
    fi
    if printf '%s' "$check_line" | grep -qE "$pattern"; then
        emit_info "$rule_id" "$file" "$lineno" "suppressed by linter:allow-${rule_id}"
        return 0
    fi
    if [ -n "$prev_line" ] && printf '%s' "$prev_line" | grep -qE "$pattern"; then
        emit_info "$rule_id" "$file" "$lineno" "suppressed by linter:allow-${rule_id} on preceding line"
        return 0
    fi
    return 1
}

# ── diff acquisition ──────────────────────────────────────────────────────────

TMP_DIFF="$(mktemp -t lint-impl-diff.XXXXXX)"
TMP_ISSUE="$(mktemp -t lint-impl-issue.XXXXXX)"
TMP_CONTRACT_OUT="$(mktemp -t lint-impl-contract-out.XXXXXX)"
TMP_CONTRACT_ERR="$(mktemp -t lint-impl-contract-err.XXXXXX)"
trap 'rm -f "$TMP_DIFF" "$TMP_ISSUE" "$TMP_CONTRACT_OUT" "$TMP_CONTRACT_ERR"' EXIT INT TERM

# Validate operator-supplied offline evidence before any successful early return.
OFFLINE_ISSUE_BODY=0
if [ "${AUTOSPEC_LINT_ISSUE_BODY_FILE+x}" = x ]; then
    if [ -z "$AUTOSPEC_LINT_ISSUE_BODY_FILE" ] ||
        [ ! -f "$AUTOSPEC_LINT_ISSUE_BODY_FILE" ] ||
        [ ! -r "$AUTOSPEC_LINT_ISSUE_BODY_FILE" ] ||
        [ -L "$AUTOSPEC_LINT_ISSUE_BODY_FILE" ]; then
        printf 'lint-implementation.sh: offline issue body file must be a readable regular file\n' >&2
        exit 1
    fi
    OFFLINE_ISSUE_BODY=1
fi

if [ -n "$DIFF_FILE" ]; then
    cp "$DIFF_FILE" "$TMP_DIFF"
elif [ "$STAGED" -eq 1 ]; then
    # Pre-commit / staged mode: read from git diff --cached
    if [ -n "$STAGED_BASE" ]; then
        git diff --cached "$STAGED_BASE" -- > "$TMP_DIFF" 2>/dev/null
    else
        git diff --cached > "$TMP_DIFF" 2>/dev/null
    fi || {
        printf 'ERROR: failed to get staged diff (git diff --cached)\n' >&2
        exit 1
    }
    if [ ! -s "$TMP_DIFF" ]; then
        printf 'lint-implementation.sh: no staged changes found\n' >&2
        exit 0
    fi
else
    # Fetch diff from GitHub
    gh pr diff "$PR_NUMBER" > "$TMP_DIFF" 2>/dev/null || {
        printf 'ERROR: failed to fetch diff for PR %s\n' "$PR_NUMBER" >&2
        exit 1
    }
fi

# Fetch issue body for skip-directive parsing in every diff source mode.
if [ -n "$ISSUE_NUMBER" ]; then
    if [ "$OFFLINE_ISSUE_BODY" -eq 1 ]; then
        cp "$AUTOSPEC_LINT_ISSUE_BODY_FILE" "$TMP_ISSUE"
    else
        gh issue view "$ISSUE_NUMBER" --json body --jq '.body' > "$TMP_ISSUE" 2>/dev/null || true
    fi
    parse_skip_directives "$TMP_ISSUE"
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

    # COMPLEXITY is advisory unless enforcement is asked for: the limits are a rough
    # heuristic, and as a veto they froze oversized files against even a one-line safe
    # edit (#2961). Deciding it here rather than at the 12 call sites means a rule added
    # later cannot forget the policy. PR_SIZE and the CI file-size ratchet stay blocking
    # — docs/superpowers/specs/2026-08-05-lint-gate-satisfiability-design.md Fix 5.
    if is_skipped "$rule_id" ||
       { [ "$rule_id" = COMPLEXITY ] && [ "${AUTOSPEC_COMPLEXITY_ENFORCE:-0}" != 1 ]; }; then
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

# ── PR_SIZE detector ─────────────────────────────────────────────────────────

PR_SIZE_MAX_LINES=400
PR_SIZE_MAX_FILES=8
PR_SIZE_MAX_UNITS=3

pr_size_file_stat() {
    local numstat
    numstat="$(mktemp -t lint-pr-size-numstat.XXXXXX)"
    if git apply --numstat "$TMP_DIFF" > "$numstat" 2>/dev/null; then
        awk -F '\t' '{
            binary=($1 == "-" || $2 == "-") ? 1 : 0
            added=binary ? 0 : $1
            removed=binary ? 0 : $2
            print added, removed, binary, $3
        }' "$numstat"
        rm -f "$numstat"
        return
    fi
    rm -f "$numstat"
    # Git rejects abbreviated synthetic binary fixtures; retain fail-closed
    # parsing for an explicit binary marker even when no full patch is present.
    awk '
        /^diff --git / {
            if (path != "") print added, removed, binary, path
            path=$0; sub(/^diff --git a\/[^ ]* b\//, "", path)
            added=0; removed=0; binary=0; hunk=0; next
        }
        /^Binary files / || /^GIT binary patch$/ { binary=1; next }
        /^@@ / { hunk=1; next }
        hunk && /^\+\+\+ / { next }
        hunk && /^--- / { next }
        hunk && /^\+/ { added++; next }
        hunk && /^-/ { removed++; next }
        END { if (path != "") print added, removed, binary, path }
    ' "$TMP_DIFF"
}

pr_size_unit() {
    local path="$1"
    case "$path" in
        tests/fixtures/skill-goldens/*.sha256) return ;;
        skills/*/SKILL.md|skills/*/codex/prompt.md|skills/*/opencode/agent.md)
            printf 'skills/%s/<trio>\n' "$(printf '%s' "$path" | cut -d/ -f2)"
            ;;
        *) printf '%s\n' "$path" ;;
    esac
}

pr_size_golden_skill() {
    local name="${1#tests/fixtures/skill-goldens/}"
    case "$name" in
        *.SKILL.md.sha256) printf '%s\n' "${name%.SKILL.md.sha256}" ;;
        *.codex.prompt.md.sha256) printf '%s\n' "${name%.codex.prompt.md.sha256}" ;;
        *.opencode.agent.md.sha256) printf '%s\n' "${name%.opencode.agent.md.sha256}" ;;
        *) return 1 ;;
    esac
}

pr_size_lock_skill() {
    case "$1" in
        skills/*/SKILL.md|skills/*/codex/prompt.md|skills/*/opencode/agent.md)
            printf '%s\n' "$(printf '%s' "$1" | cut -d/ -f2)"
            ;;
        *) return 1 ;;
    esac
}

pr_size_fingerprint() {
    local target="$1"
    awk -v target="$target" '
        /^diff --git / {
            path=$0; sub(/^diff --git a\/[^ ]* b\//, "", path)
            inside=(path == target); hunk=0; next
        }
        inside && /^@@ / { hunk=1; print "@@"; next }
        inside && hunk && /^[ +\-]/ {
            if ($0 !~ /^\+\+\+ / && $0 !~ /^--- /) print
        }
    ' "$TMP_DIFF"
}

pr_size_is_code_or_test() {
    case "$1" in
        tests/*|*/tests/*) return 0 ;;
        *.md|*.txt|*.diff|*.json|*.yaml|*.yml) return 1 ;;
        *) return 0 ;;
    esac
}

pr_size_validate_lock_step() {
    local identity="$1" stats="$2" mirrors goldens manual skills skill expected
    mirrors="$(mktemp -t lint-pr-size-mirrors.XXXXXX)"
    goldens="$(mktemp -t lint-pr-size-goldens.XXXXXX)"
    manual="$(mktemp -t lint-pr-size-manual.XXXXXX)"
    while read -r added removed binary path; do
        if skill="$(pr_size_lock_skill "$path")"; then
            printf '%s %s %s %s %s\n' "$skill" "$added" "$removed" "$binary" "$path" >> "$mirrors"
        elif skill="$(pr_size_golden_skill "$path" 2>/dev/null)"; then
            printf '%s\n' "$skill" >> "$goldens"
        else
            printf '%s %s %s %s\n' "$added" "$removed" "$binary" "$path" >> "$manual"
        fi
    done < "$stats"
    skills="$(awk '{print $1}' "$mirrors" | sort -u)"
    [ -n "$skills" ] || { rm -f "$mirrors" "$goldens" "$manual"; return 1; }
    if [ "$(printf '%s\n' "$skills" | wc -l | tr -d ' ')" -gt 1 ]; then
        expected="$(printf '%s\n' "$skills" | paste -sd ' ' - | sed 's/ / and /g') adapter trios plus derived goldens"
        [ "$identity" = "$expected" ] || { rm -f "$mirrors" "$goldens" "$manual"; return 1; }
    else
        skill="$skills"
        identity="${identity% adapters}"
        identity="${identity#skills/}"
        [ "$identity" = "$skill" ] || { rm -f "$mirrors" "$goldens" "$manual"; return 1; }
    fi
    while IFS= read -r skill; do
        [ -z "$skill" ] && continue
        awk -v skill="$skill" '$1 == skill {found=1} END {exit !found}' "$mirrors" \
            || { rm -f "$mirrors" "$goldens" "$manual"; return 1; }
    done < "$goldens"
    while IFS= read -r skill; do
        local group first fp
        group="$(awk -v skill="$skill" '$1 == skill {print $5}' "$mirrors")"
        [ "$(printf '%s\n' "$group" | wc -l | tr -d ' ')" -eq 3 ] \
            || { rm -f "$mirrors" "$goldens" "$manual"; return 1; }
        for expected in "skills/$skill/SKILL.md" "skills/$skill/codex/prompt.md" "skills/$skill/opencode/agent.md"; do
            printf '%s\n' "$group" | grep -qxF "$expected" \
                || { rm -f "$mirrors" "$goldens" "$manual"; return 1; }
        done
        first="$(printf '%s\n' "$group" | head -1)"
        fp="$(mktemp -t lint-pr-size-fp.XXXXXX)"
        pr_size_fingerprint "$first" > "$fp"
        while IFS= read -r path; do
            cmp -s "$fp" <(pr_size_fingerprint "$path") \
                || { rm -f "$fp" "$mirrors" "$goldens" "$manual"; return 1; }
        done <<< "$group"
        rm -f "$fp"
    done <<< "$skills"
    while read -r _ _ _ path; do
        pr_size_is_code_or_test "$path" \
            && { rm -f "$mirrors" "$goldens" "$manual"; return 1; }
    done < "$manual"
    local lines files units unit unit_file
    lines="$(awk '{sum += $1 + $2} END {print sum + 0}' "$manual")"
    files="$(wc -l < "$manual" | tr -d ' ')"
    unit_file="$(mktemp -t lint-pr-size-units.XXXXXX)"
    while read -r _ _ _ path; do pr_size_unit "$path" >> "$unit_file"; done < "$manual"
    while IFS= read -r skill; do
        read -r added removed _ _ < <(awk -v skill="$skill" '$1 == skill {print $2, $3, $4, $5; exit}' "$mirrors")
        lines=$((lines + added + removed)); files=$((files + 1))
        printf 'skills/%s/<trio>\n' "$skill" >> "$unit_file"
    done <<< "$skills"
    units="$(sort -u "$unit_file" | sed '/^$/d' | wc -l | tr -d ' ')"
    rm -f "$unit_file" "$mirrors" "$goldens" "$manual"
    [ "$lines" -le "$PR_SIZE_MAX_LINES" ] && [ "$files" -le "$PR_SIZE_MAX_FILES" ] \
        && [ "$units" -le "$PR_SIZE_MAX_UNITS" ]
}

pr_size_validate_exception() {
    local category="$1" detail="$2" stats="$3" path
    awk '$3 == 1 {found=1} END {exit !found}' "$stats" && return 1
    case "$category" in
        "generated migration")
            while read -r _ _ _ path; do
                printf '%s\n' "$path" | grep -qE '(^|/)(migration|migrations|migrate)(/|$)' || return 1
                get_added_lines_for_file "$path" | tr '[:upper:]' '[:lower:]' \
                    | grep -F "generated" | grep -Fq "$(printf '%s' "$detail" | tr '[:upper:]' '[:lower:]')" \
                    || return 1
            done < "$stats"
            ;;
        "dependency-solver lockfile")
            while read -r _ _ _ path; do
                local name="${path##*/}"
                case "$detail:$name" in
                    bundler:Gemfile.lock|cargo:Cargo.lock|composer:composer.lock|go:go.sum|\
                    gradle:gradle.lockfile|npm:package-lock.json|npm:npm-shrinkwrap.json|\
                    pipenv:Pipfile.lock|pnpm:pnpm-lock.yaml|poetry:poetry.lock|yarn:yarn.lock) ;;
                    *) return 1 ;;
                esac
            done < "$stats"
            ;;
        "mandatory lock-step artifacts") pr_size_validate_lock_step "$detail" "$stats" ;;
        *) return 1 ;;
    esac
}

detect_pr_size() {
    local stats lines files units binary exceeded="" unit_file category="" detail="" exception=""
    stats="$(mktemp -t lint-pr-size-stats.XXXXXX)"
    unit_file="$(mktemp -t lint-pr-size-units.XXXXXX)"
    pr_size_file_stat > "$stats"
    lines="$(awk '{sum += $1 + $2} END {print sum + 0}' "$stats")"
    files="$(wc -l < "$stats" | tr -d ' ')"
    while read -r _ _ _ path; do pr_size_unit "$path" >> "$unit_file"; done < "$stats"
    units="$(sort -u "$unit_file" | sed '/^$/d' | wc -l | tr -d ' ')"
    binary=false
    awk '$3 == 1 {found=1} END {exit !found}' "$stats" && binary=true
    [ "$lines" -gt "$PR_SIZE_MAX_LINES" ] && exceeded="changed_lines"
    [ "$files" -gt "$PR_SIZE_MAX_FILES" ] && exceeded="${exceeded:+$exceeded,}raw_files"
    [ "$units" -gt "$PR_SIZE_MAX_UNITS" ] && exceeded="${exceeded:+$exceeded,}logical_units"
    [ "$binary" = true ] && exceeded="${exceeded:+$exceeded,}binary"
    rm -f "$unit_file"
    [ -n "$exceeded" ] || { rm -f "$stats"; return; }
    if [ -s "$TMP_ISSUE" ]; then
        exception="$(grep -m1 '^Guardian: skip-PR_SIZE # ' "$TMP_ISSUE" || true)"
        exception="${exception#Guardian: skip-PR_SIZE # }"
        category="${exception%%:*}"
        [ "$category" != "$exception" ] && detail="${exception#*:}" && detail="${detail#"${detail%%[![:space:]]*}"}" && detail="${detail%"${detail##*[![:space:]]}"}"
    fi
    local message="changed_lines=$lines/$PR_SIZE_MAX_LINES raw_files=$files/$PR_SIZE_MAX_FILES logical_units=$units/$PR_SIZE_MAX_UNITS binary=$binary exceeded=$exceeded"
    if [ -n "$detail" ] && pr_size_validate_exception "$category" "$detail" "$stats"; then
        emit_info "PR_SIZE" "-" "-" "$message category=$category"
    else
        emit_error "PR_SIZE" "-" "-" "$message"
    fi
    rm -f "$stats"
}

# get_added_lines_for_file FILE — print added lines (without leading +) for a file
# Scans the diff hunk by hunk, limiting to the given file section.
get_added_lines_for_file() {
    local target="$1"
    awk -v tgt="$target" '
        /^diff --git / {
            path=substr($0, 12); sub(/^a\/.* b\//, "", path); in_file=(path == tgt)
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

# ── §3.1 issue implementation-contract adapter ────────────────────────────────
# Delegates only OUT_OF_SCOPE and MISSING_TEST classification to the Rust CLI,
# then re-emits through the shell accumulator so ordering, skips, and caps remain
# owned by this process.

contract_cli_failure() {
    if [ "$DIRECTIVES" -eq 1 ]; then
        printf 'lint-implementation.sh: %s\n' "$1" >&3
    else
        printf 'lint-implementation.sh: %s\n' "$1" >&2
    fi
    exit 1
}

resolve_implementation_contract_cli() {
    local configured="${AUTOSPEC_BIN:-}"
    if [ -n "$configured" ]; then
        [ -f "$configured" ] && [ -x "$configured" ] || return 1
        printf '%s\n' "$configured"
        return 0
    fi

    local candidate="$SCRIPT_DIR/../target/debug/autospec"
    if [ -f "$candidate" ] && [ -x "$candidate" ]; then
        printf '%s\n' "$candidate"
        return 0
    fi

    candidate="$(command -v autospec 2>/dev/null || true)"
    if [ -n "$candidate" ] && [ -f "$candidate" ] && [ -x "$candidate" ]; then
        printf '%s\n' "$candidate"
        return 0
    fi

    if [ -n "${HOME:-}" ]; then
        candidate="$HOME/.autospec/bin/autospec"
        if [ -f "$candidate" ] && [ -x "$candidate" ]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    fi
    return 1
}

detect_implementation_contract() {
    if [ ! -s "$TMP_ISSUE" ]; then
        return 0
    fi

    local contract_cli=""
    contract_cli="$(resolve_implementation_contract_cli || true)"
    if [ -z "$contract_cli" ]; then
        contract_cli_failure "implementation-contract CLI is unavailable: autospec"
    fi

    : > "$TMP_CONTRACT_OUT"
    : > "$TMP_CONTRACT_ERR"
    local cli_status=0
    "$contract_cli" lint implementation-contract \
        --issue-body-file "$TMP_ISSUE" \
        --diff-file "$TMP_DIFF" \
        > "$TMP_CONTRACT_OUT" 2> "$TMP_CONTRACT_ERR" || cli_status=$?

    validate_implementation_contract_output "$cli_status"
    while IFS= read -r finding; do
        [ -z "$finding" ] && continue
        local payload="${finding#INFO:}"
        local rule_id="${payload%%:*}"
        payload="${payload#*:}"
        local path="${payload%%:*}"
        payload="${payload#*:}"
        local line="${payload%%:*}"
        local desc="${payload#*: }"
        emit_capped "$rule_id" "$path" "$line" "$desc"
    done < "$TMP_CONTRACT_OUT"
}

validate_implementation_contract_output() {
    local cli_status="$1"
    local blocking_count=0
    local output_count=0
    while IFS= read -r finding; do
        [ -z "$finding" ] && continue
        output_count=$((output_count + 1))
        if ! printf '%s\n' "$finding" \
            | grep -qE '^(INFO:)?(OUT_OF_SCOPE|MISSING_TEST):[^:]+:(-|[0-9]+): .+'; then
            contract_cli_failure "malformed implementation-contract CLI output: $finding"
        fi
        case "$finding" in
            INFO:*)
                local info_rule="${finding#INFO:}"
                info_rule="${info_rule%%:*}"
                if ! is_skipped "$info_rule"; then
                    contract_cli_failure "unexpected INFO finding from implementation-contract CLI: $finding"
                fi
                ;;
            *) blocking_count=$((blocking_count + 1)) ;;
        esac
    done < "$TMP_CONTRACT_OUT"

    if [ "$cli_status" -ne 0 ] && [ "$output_count" -eq 0 ]; then
        local detail="no diagnostic"
        if [ -s "$TMP_CONTRACT_ERR" ]; then
            detail="$(cat "$TMP_CONTRACT_ERR")"
        fi
        contract_cli_failure "implementation-contract CLI failed without findings (exit ${cli_status}): ${detail}"
    fi
    if [ -s "$TMP_CONTRACT_ERR" ]; then
        contract_cli_failure "implementation-contract CLI failed: $(cat "$TMP_CONTRACT_ERR")"
    fi
    if [ "$cli_status" -ne "$blocking_count" ]; then
        contract_cli_failure "implementation-contract CLI exit ${cli_status} disagrees with ${blocking_count} blocking finding(s)"
    fi
}

# ── §3.1 COMPLEXITY detector ──────────────────────────────────────────────────
# Function >50 LOC, file >500 LOC, nesting >4.

# py_ast_nesting_findings DIFF_FILE
# Issue #1245: For Python files, the leading-spaces/4 indentation proxy badly
# over-counts nesting — module docstrings, multi-line call-argument
# continuations (e.g. argparse.add_argument), and dict/list literals all look
# like deep code nesting but carry no control-flow depth. We instead reconstruct
# the file's added source and walk it with python3's `ast` module, measuring the
# real maximum nesting of control-flow / scope nodes (If/For/While/With/Try/
# FunctionDef and their async variants). Per offending function (real depth >4)
# we emit COMPLEXITY at the function's def line with the true depth.
#
# Mirrors check_cyclomatic's optional-python3 pattern: this is only attempted
# when `python3` is available AND the source parses. On either failure the
# caller falls back to the indentation proxy (see detect_complexity).
#
# Returns 0 if the AST analysis ran (findings emitted as a side effect),
# non-zero if python3 is unavailable or the source failed to parse — signalling
# the caller to fall back to the indentation proxy.
py_ast_nesting_findings() {
    local diff_file="$1"
    command -v python3 >/dev/null 2>&1 || return 1

    local src
    src="$(get_added_lines_for_file "$diff_file")"
    [ -n "$src" ] || return 1

    # Feed source on stdin; emit "LINENO\tDEPTH" lines for functions whose real
    # control-flow nesting depth exceeds the threshold (4). Exit non-zero on a
    # SyntaxError so the caller falls back to the proxy.
    local out
    out="$(printf '%s\n' "$src" | python3 -c '
import ast, sys

THRESHOLD = 4
NEST = (ast.If, ast.For, ast.While, ast.With, ast.Try,
        ast.FunctionDef, ast.AsyncFunctionDef)
# AsyncFor / AsyncWith exist on all supported python3 versions.
NEST = NEST + (ast.AsyncFor, ast.AsyncWith)

try:
    tree = ast.parse(sys.stdin.read())
except SyntaxError:
    sys.exit(3)

# Record, per enclosing function def, the maximum nesting depth reached within
# it. Depth counts the control-flow / scope nodes on the path (docstrings,
# string/dict/list literals and call continuations are NOT ast control nodes,
# so they never contribute). The function def itself counts as depth 1.
findings = {}

def walk(node, depth, func_lineno):
    for child in ast.iter_child_nodes(node):
        if isinstance(child, NEST):
            d = depth + 1
            fl = func_lineno
            if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef)):
                fl = child.lineno
            if fl is not None and d > THRESHOLD:
                prev = findings.get(fl, 0)
                if d > prev:
                    findings[fl] = d
            walk(child, d, fl)
        else:
            walk(child, depth, func_lineno)

walk(tree, 0, None)
for lineno in sorted(findings):
    sys.stdout.write("%d\t%d\n" % (lineno, findings[lineno]))
' 2>/dev/null)"
    local rc=$?
    [ "$rc" -eq 0 ] || return 1

    # One finding per offending function. Here-document, not pipe — see Fix 7 (#3081).
    while IFS="$(printf '\t')" read -r fline fdepth; do
        [ -z "$fline" ] && continue
        emit_capped "COMPLEXITY" "$diff_file" "$fline" "nesting depth ${fdepth} (threshold: 4) at line ${fline}"
    done <<EOF
$out
EOF
    return 0
}

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

        # Nesting depth: for Python (#1245), prefer a real AST control-flow
        # depth over the leading-spaces/4 proxy. If python3 is unavailable or
        # the source fails to parse, py_ast_nesting_findings returns non-zero
        # and we leave _nesting_proxy=1 so the indentation proxy still runs.
        local _nesting_proxy=1
        case "$diff_file" in
            *.py)
                if py_ast_nesting_findings "$diff_file"; then
                    _nesting_proxy=0
                fi
                ;;
        esac

        # Function LOC check: scan for function definitions, count body lines
        # Strategy: track function start/end in added lines
        local in_func=0
        local func_start=0
        local func_name=""
        local func_loc=0
        local line_no=0

        # Heredoc tracking (issue #656): physical indentation inside heredoc
        # bodies is string-literal content, not code nesting. We track the
        # active heredoc marker line-by-line and skip the nesting-depth
        # measurement until the matching closer is seen.
        # heredoc_marker = active terminator string ("" when not inside one)
        # heredoc_dash   = 1 if opener used `<<-MARKER` (closer may be indented)
        local heredoc_marker=""
        local heredoc_dash=0
        # Bash-regex equivalents of the greps below (hoisted to avoid
        # rebuilding per line; no subprocess is spawned for any of these).
        local heredoc_open_pat='<<(-)?[[:space:]]*(\\|'\''|")?([A-Za-z_][A-Za-z0-9_]*)'
        local func_start_pat='^[[:space:]]*(function[[:space:]]+)?[A-Za-z_][A-Za-z0-9_]*\(\)[[:space:]]*\{?[[:space:]]*$'
        local first_token_pat='[A-Za-z_][A-Za-z0-9_]*'
        local close_brace_pat='^}[[:space:]]*$'

        while IFS=: read -r lineno content; do
            line_no="$lineno"

            # If currently inside a heredoc, check whether this line is the
            # matching closer; either way, skip code-structure measurement.
            if [ -n "$heredoc_marker" ]; then
                # Plain << form: closer must be at column 0. <<- form: bash
                # strips ONLY leading tabs from body and closer (POSIX);
                # spaces before the marker do NOT close it.
                local closer_pat="^${heredoc_marker}[[:space:]]*$"
                [ "$heredoc_dash" -eq 1 ] && closer_pat="^	*${heredoc_marker}[[:space:]]*$"
                if [[ "$content" =~ $closer_pat ]]; then
                    heredoc_marker=""
                    heredoc_dash=0
                fi
                # While inside heredoc body, body lines are string literals —
                # don't measure nesting and don't try to detect function starts
                # (heredoc text can contain `name() {` patterns).
                if [ "$in_func" -eq 1 ]; then
                    func_loc=$((func_loc + 1))
                fi
                continue
            fi

            # Detect a new heredoc opener on this line. We accept the common
            # forms `<<MARKER`, `<<-MARKER`, `<<'MARKER'`, `<<"MARKER"`,
            # `<<\MARKER`. Capture the dash flag and bare marker directly.
            if [[ "$content" =~ $heredoc_open_pat ]]; then
                if [ -n "${BASH_REMATCH[1]}" ]; then heredoc_dash=1; else heredoc_dash=0; fi
                heredoc_marker="${BASH_REMATCH[3]}"
                # Fall through: still let this line participate in function/
                # nesting detection because the opener itself is real code.
            fi

            # Detect function start (bash-style: name() { or function name {)
            if [[ "$content" =~ $func_start_pat ]]; then
                if [ "$in_func" -eq 1 ] && [ "$func_loc" -gt 50 ]; then
                    emit_capped "COMPLEXITY" "$diff_file" "$func_start" "function '${func_name}' is ${func_loc} LOC (threshold: 50)"
                fi
                in_func=1
                func_start="$lineno"
                if [[ "$content" =~ $first_token_pat ]]; then func_name="${BASH_REMATCH[0]}"; else func_name=""; fi
                func_loc=0
                continue
            fi

            if [ "$in_func" -eq 1 ]; then
                # Detect closing brace at column 0 (end of function)
                if [[ "$content" =~ $close_brace_pat ]]; then
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

            # Nesting depth check: count leading spaces / 4 as proxy for depth.
            # Skipped for Python files already handled by the AST analysis
            # above (_nesting_proxy=0); still used for .sh/.mjs and as the
            # python3-absent / parse-failure fallback.
            if [ "$_nesting_proxy" -eq 1 ]; then
                local stripped="${content%%[^ ]*}"
                local depth=$((${#stripped} / 4))
                if [ "$depth" -gt 4 ]; then
                    emit_capped "COMPLEXITY" "$diff_file" "$lineno" "nesting depth ~${depth} (threshold: 4) at line ${lineno}"
                fi
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

# scan_security_pattern PAT DESC — scan added lines of every diff file for PAT. One awk
# pass per file plus an in-process bash regex test: no per-line grep/sed subprocess.
scan_security_pattern() {
    local pat="$1"
    local desc="$2"
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        while IFS=: read -r lineno content; do
            # +1 keeps the pre-existing (off-by-one) numbering, and is the line
            # is_line_allowed reads — annotate the offending line itself (#3057).
            if [[ "$content" =~ $pat ]] && ! is_line_allowed SECURITY "$diff_file" "$((lineno + 1))"; then
                emit_capped "SECURITY" "$diff_file" "$((lineno + 1))" "$desc"
            fi
        done <<EOF
$(get_added_lines_with_lineno "$diff_file")
EOF
    done <<EOF
$(get_diff_files)
EOF
}

detect_security() {
    # Boundary-anchored: ast.literal_eval is not the builtin, but builtins.eval is (Fix 6).
    scan_security_pattern '(^|[^A-Za-z0-9_])eval\(' "eval() usage — potential code injection"  # linter:allow-SECURITY the rule's own pattern and message
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
    # Built via concatenation so this pattern definition itself never contains
    # a contiguous deferred-work marker (this file is non-test source, so its
    # own diff hunks are scanned by this same detector).
    local pat='\b(TOD''O|XX''X|FIXM''E)\b'
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        is_test_file "$diff_file" && continue

        while IFS=: read -r lineno content; do
            if [[ "$content" =~ $pat ]]; then
                emit_capped "TODO_LEFT" "$diff_file" "$lineno" "${BASH_REMATCH[1]} found in non-test source"
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
            local is_mock=0 is_db=0
            shopt -s nocasematch
            [[ "$content" =~ $mock_pattern ]] && is_mock=1
            [[ "$content" =~ $db_pattern ]] && is_db=1
            shopt -u nocasematch
            if [ "$is_mock" -eq 1 ] && [ "$is_db" -eq 1 ]; then
                emit_capped "MOCK_DB" "$diff_file" "$lineno" "mock/stub of DB symbol detected in test"
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
    local cli_pat='(^|[[:space:]])(--[a-z][a-z0-9-]{2,})[[:space:]=]'
    local env_pat='^(export[[:space:]]+)?[A-Z][A-Z0-9]*_[A-Z0-9_]+='
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        is_test_file "$diff_file" && continue
        is_doc_file "$diff_file" && continue

        # Skip binary-ish files, and Rust tests: is_test_file only matches a repo-root tests/
        case "$diff_file" in
            *.diff|*.png|*.jpg|*.gif|*/tests/*.rs|*/tests.rs) continue ;;
        esac

        while IFS=: read -r lineno content; do
            # CLI flag pattern: --flag-name with at least 3 chars (avoid -f style short flags in comments)
            if [[ "$content" =~ $cli_pat ]]; then
                emit_capped "DOC_OUT_OF_SYNC" "$diff_file" "$lineno" "CLI long-flag introduced without touching a doc file"
                break
            fi
            # Env var export pattern: export FOO_BAR= or FOO_BAR= at start of line (require underscore or 3+ caps)
            if [[ "$content" =~ $env_pat ]]; then
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

# ── §3.1 VACUOUS_* detectors ─────────────────────────────────────────────────
# Detects 8 vacuous-test patterns where assertions always pass regardless of behavior.
# Active when --vacuous-assertions or --pre-commit flag is set.

# _vacuous_grep_or_true FILE LINENO CONTENT — check GREP_INVERSE and OR_TRUE patterns
_vacuous_grep_or_true() {
    local diff_file="$1" lineno="$2" content="$3"
    local grep_inv_pat='grep -qv .* \|\| true'
    if [[ "$content" =~ $grep_inv_pat ]]; then
        emit_capped "VACUOUS_GREP_INVERSE_OR_TRUE" "$diff_file" "$lineno" \
            "\`grep -qv\` with \`|| true\` is a no-op assertion. Use \`! grep -q\` instead."
        return
    fi
    # VACUOUS_OR_TRUE: || true at end of line — only flag in test files
    if is_test_file "$diff_file"; then
        local or_true_pat='\|\| true[[:space:]]*$'
        if [[ "$content" =~ $or_true_pat ]]; then
            emit_capped "VACUOUS_OR_TRUE" "$diff_file" "$lineno" \
                "\`|| true\` at end of assertion masks failure — assertion always exits 0."
        fi
    fi
}

# _vacuous_tautology_and_stubs FILE LINENO CONTENT — TAUTOLOGY, AC_STUB, EMPTY_TEST. The xit pattern is anchored; unanchored it also matched sys.exit and SystemExit.
_vacuous_tautology_and_stubs() {
    local diff_file="$1" lineno="$2" content="$3"
    local taut_pat='expect\((true|1)\)\.(toBe|toEqual|toStrictEqual)\(\1\)|assert\(1\s*===?\s*1\)|assert True[[:space:]]*$|(^|[^A-Za-z0-9_.])xit\(|assert\.ok\(true\)|t\.true\(true\)'
    if [[ "$content" =~ $taut_pat ]] && ! is_line_allowed VACUOUS_TAUTOLOGY "$diff_file" "$lineno"; then
        emit_capped "VACUOUS_TAUTOLOGY" "$diff_file" "$lineno" \
            "Tautological assertion — always passes regardless of code under test."
    fi
    case "$diff_file" in
        tests/ac/*)
            local stub_pat='skip[[:space:]]+"?auto-stub"?'
            if [[ "$content" =~ $stub_pat ]]; then
                emit_capped "VACUOUS_AC_STUB" "$diff_file" "$lineno" \
                    "Auto-generated stub test with skip — replace with a real assertion."
            fi ;;
    esac
    local empty_js='it\([[:space:]]*["'"'"'][^"'"'"']+["'"'"'][[:space:]]*,[[:space:]]*\(\)[[:space:]]*=>[[:space:]]*\{[[:space:]]*\}'
    local empty_bats='^[[:space:]]*@test[[:space:]]+"[^"]+"[[:space:]]*\{[[:space:]]*\}[[:space:]]*$'
    if [[ "$content" =~ $empty_js ]]; then
        emit_capped "VACUOUS_EMPTY_TEST" "$diff_file" "$lineno" \
            "Empty test body — it() callback has no assertions."
    fi
    if [[ "$content" =~ $empty_bats ]]; then
        emit_capped "VACUOUS_EMPTY_TEST" "$diff_file" "$lineno" \
            "Empty bats @test body — no assertions."
    fi
}

# _vacuous_scan_file FILE — dispatch per-line vacuous pattern checks
_vacuous_scan_file() {
    local diff_file="$1"
    while IFS=: read -r lineno content; do
        _vacuous_grep_or_true "$diff_file" "$lineno" "$content"
        _vacuous_tautology_and_stubs "$diff_file" "$lineno" "$content"
    done <<EOF
$(get_added_lines_with_lineno "$diff_file")
EOF
}

# _vacuous_scan_no_assert FILE — warn when a test file has added test blocks with no assert/expect
# _vacuous_emit_no_assert — emit VACUOUS_NO_ASSERT if test has no assertions
_vacuous_emit_no_assert() {
    local diff_file="$1" test_start="$2" test_name="$3" has_assert="$4"
    if [ "$has_assert" -eq 0 ] && [ -n "$test_name" ]; then
        emit_capped "VACUOUS_NO_ASSERT" "$diff_file" "$test_start" \
            "Test '${test_name}' has no assert/run/grep assertion (WARN)."
    fi
}

# _vacuous_scan_no_assert FILE — warn when added bats test blocks lack assertions
_vacuous_scan_no_assert() {
    local diff_file="$1"
    local in_test=0 test_start=0 has_assert=0 test_name=""
    local start_pat='^[[:space:]]*@test[[:space:]]'
    local name_pat='"([^"]+)"'
    local close_pat='^[[:space:]]*\}[[:space:]]*$'
    local assert_pat='\b(assert|expect|run|grep|check|verify)\b'

    while IFS=: read -r lineno content; do
        # New @test block: flush previous if open
        if [[ "$content" =~ $start_pat ]]; then
            if [ "$in_test" -eq 1 ]; then
                _vacuous_emit_no_assert "$diff_file" "$test_start" "$test_name" "$has_assert"
            fi
            in_test=1; has_assert=0; test_start="$lineno"
            if [[ "$content" =~ $name_pat ]]; then test_name="${BASH_REMATCH[1]}"; else test_name=""; fi
            continue
        fi
        [ "$in_test" -eq 0 ] && continue
        # Closing brace: flush and reset
        if [[ "$content" =~ $close_pat ]]; then
            _vacuous_emit_no_assert "$diff_file" "$test_start" "$test_name" "$has_assert"
            in_test=0; has_assert=0; test_name=""; continue
        fi
        # Assert/expect/run/grep counts as assertion presence
        if [[ "$content" =~ $assert_pat ]]; then
            has_assert=1
        fi
    done <<EOF
$(get_added_lines_with_lineno "$diff_file")
EOF
    # Flush last open test block
    if [ "$in_test" -eq 1 ]; then
        _vacuous_emit_no_assert "$diff_file" "$test_start" "$test_name" "$has_assert"
    fi
}

detect_vacuous_assertions() {
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        # Only scan test files and source files (skip docs/fixtures)
        case "$diff_file" in
            *.md|*.txt|*.diff|*.json|*.yaml|*.yml) continue ;;
        esac
        _vacuous_scan_file "$diff_file"
        # VACUOUS_NO_ASSERT only for test files
        if is_test_file "$diff_file"; then
            _vacuous_scan_no_assert "$diff_file"
        fi
    done <<EOF
$(get_diff_files)
EOF
}

# ── §3.5 Assertion-density floor detector ────────────────────────────────────
# Active when --assertion-density or --pre-commit flag is set.
# Flags test blocks (bats @test / JS it() / Python def test_) that have zero
# assert/expect/run/grep calls. Emits ASSERTION_DENSITY:<file>:<line>: <desc>

# _density_flush RULE FILE LINE HAS_ASSERT IN_BLOCK_REF HAS_ASSERT_REF — emit if no assertion
_density_flush() {
    local diff_file="$1" block_start="$2" has_assert="$3"
    if [ "$has_assert" -eq 0 ]; then
        emit_capped "ASSERTION_DENSITY" "$diff_file" "$block_start" \
            "test block has no assert/expect/run/grep call — add a real assertion"
    fi
}

# _density_scan_file DIFF_FILE — scan added lines for zero-assertion test blocks
_density_scan_file() {
    local diff_file="$1"
    local in_block=0 block_start=0 has_assert=0 block_lang="" lineno content
    local bats_pat='^[+]?[[:space:]]*@test[[:space:]]+"'
    local js_pat='^[+]?[[:space:]]*(it|test)[[:space:]]*\('
    local py_pat='^[+]?[[:space:]]*def[[:space:]]+test_'
    local assert_pat='\b(assert|expect|run|grep|check|verify|assertEqual|assertIn|assertTrue|assertFalse|assertRaises)\b'
    local brace_pat='^[+]?[[:space:]]*\}[[:space:]]*$'
    while IFS=: read -r lineno content; do
        # bats @test block start
        if [[ "$content" =~ $bats_pat ]]; then
            [ "$in_block" -eq 1 ] && _density_flush "$diff_file" "$block_start" "$has_assert"
            in_block=1; block_start="$lineno"; has_assert=0; block_lang="bats"; continue
        fi
        # JS/TS it()/test() block start
        if [[ "$content" =~ $js_pat ]]; then
            [ "$in_block" -eq 1 ] && _density_flush "$diff_file" "$block_start" "$has_assert"
            in_block=1; block_start="$lineno"; has_assert=0; block_lang="js"; continue
        fi
        # Python def test_ block start
        if [[ "$content" =~ $py_pat ]]; then
            [ "$in_block" -eq 1 ] && _density_flush "$diff_file" "$block_start" "$has_assert"
            in_block=1; block_start="$lineno"; has_assert=0; block_lang="python"; continue
        fi
        [ "$in_block" -eq 0 ] && continue
        # Assertion presence check
        if [[ "$content" =~ $assert_pat ]]; then
            has_assert=1
        fi
        # Bats block end on closing brace
        if [ "$block_lang" = "bats" ] && [[ "$content" =~ $brace_pat ]]; then
            _density_flush "$diff_file" "$block_start" "$has_assert"
            in_block=0; has_assert=0
        fi
    done <<EOF
$(get_added_lines_with_lineno "$diff_file")
EOF
    # Flush last open block
    if [ "$in_block" -eq 1 ]; then
        _density_flush "$diff_file" "$block_start" "$has_assert"
    fi
}

detect_assertion_density() {
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        if ! is_test_file "$diff_file"; then continue; fi
        case "$diff_file" in
            *.md|*.txt|*.diff|*.json|*.yaml|*.yml) continue ;;
        esac
        _density_scan_file "$diff_file"
    done <<EOF
$(get_diff_files)
EOF
}

# ── §3.x Deterministic complexity gates ──────────────────────────────────────
# Configurable via AUTOSPEC_MAX_FILE_LOC, AUTOSPEC_MAX_FUNC_LOC,
# AUTOSPEC_MAX_CYCLOMATIC env vars.  Only changed files are examined.

_COMPLEXITY_MAX_FILE_LOC="${AUTOSPEC_MAX_FILE_LOC:-400}"
_COMPLEXITY_MAX_FUNC_LOC="${AUTOSPEC_MAX_FUNC_LOC:-50}"
_COMPLEXITY_MAX_CYCLOMATIC="${AUTOSPEC_MAX_CYCLOMATIC:-10}"

# check_file_loc — emit COMPLEXITY finding if a changed file exceeds max LOC.
check_file_loc() {
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        [ -f "$diff_file" ] || continue
        case "$diff_file" in
            *.md|*.txt|*.json|*.yaml|*.yml|*.diff) continue ;;
        esac
        local loc
        loc="$(wc -l < "$diff_file" | tr -d ' ')"
        if [ "$loc" -gt "$_COMPLEXITY_MAX_FILE_LOC" ]; then
            emit_capped "COMPLEXITY" "$diff_file" "-" "file is ${loc} LOC (AUTOSPEC_MAX_FILE_LOC=${_COMPLEXITY_MAX_FILE_LOC}); split into smaller modules"
        fi
    done <<EOF
$(get_diff_files)
EOF
}

# check_function_loc — emit COMPLEXITY finding for functions exceeding max LOC, by full-file
# analysis. Shell functions are covered by detect_complexity instead, from the diff. Python
# ONLY: .ts/.js/.go were admitted by the extension filter and then fell through the
# Python-only body, so they were never analysed — the filter now says what the code does.
check_function_loc() {
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        [ -f "$diff_file" ] || continue
        case "$diff_file" in *.py) ;; *) continue ;; esac
        # def <name>( — lines until the next def/class. Here-document, not pipe, so the
        # counters survive (Fix 7); func_name below is awk's, not the shell's.
        while IFS=: read -r fname fstart floc; do
            [ -z "$fname" ] && continue
            emit_capped "COMPLEXITY" "$diff_file" "$fstart" "function '${fname}' is ${floc} LOC (AUTOSPEC_MAX_FUNC_LOC=${_COMPLEXITY_MAX_FUNC_LOC})"
        done <<EOF
$(awk '
                /^[[:space:]]*(def |class )[A-Za-z_]/ {
                    if (func_name && NR - func_start > max_loc) {
                        print func_name ":" func_start ":" (NR - func_start)
                    }
                    func_name=$2; func_start=NR
                }
                END {
                    if (func_name && NR - func_start > max_loc) {
                        print func_name ":" func_start ":" (NR - func_start)
                    }
                }
            ' max_loc="$_COMPLEXITY_MAX_FUNC_LOC" "$diff_file")
EOF
    done <<EOF
$(get_diff_files)
EOF
}

# check_cyclomatic — emit COMPLEXITY finding for functions with high cyclomatic
# complexity (keyword-count proxy: if/elif/else/for/while/case/catch/except).
# Uses radon for Python if available; falls back to keyword count.
check_cyclomatic() {
    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        [ -f "$diff_file" ] || continue
        case "$diff_file" in
            *.py) ;;
            *) continue ;;
        esac
        if command -v radon >/dev/null 2>&1; then
            radon cc -s "$diff_file" 2>/dev/null | \
            grep -E '[A-Z] ' | \
            while read -r _ _ _ _ score rest; do
                score_num="${score//[^0-9]/}"
                [ -n "$score_num" ] && [ "$score_num" -gt "$_COMPLEXITY_MAX_CYCLOMATIC" ] 2>/dev/null && \
                    emit_capped "COMPLEXITY" "$diff_file" "-" "cyclomatic complexity ${score_num} (AUTOSPEC_MAX_CYCLOMATIC=${_COMPLEXITY_MAX_CYCLOMATIC}) — ${rest}"
            done
        else
            # Keyword-count proxy: count decision points per function
            local count
            count="$(grep -cE '^\s+(if |elif |else:|for |while |case |except |catch )' "$diff_file" 2>/dev/null || true)"
            if [ -n "$count" ] && [ "$count" -gt "$_COMPLEXITY_MAX_CYCLOMATIC" ] 2>/dev/null; then
                emit_capped "COMPLEXITY" "$diff_file" "-" "keyword-proxy cyclomatic ~${count} (AUTOSPEC_MAX_CYCLOMATIC=${_COMPLEXITY_MAX_CYCLOMATIC}); install radon for accurate analysis"
            fi
        fi
    done <<EOF
$(get_diff_files)
EOF
}

# check_duplicate_names — emit COMPLEXITY finding for duplicate function/method
# names across changed files (likely copy-paste duplication).
check_duplicate_names() {
    local changed_files
    changed_files="$(get_diff_files | grep -E '\.(py|ts|js|go|sh)$' | tr '\n' ' ')"
    [ -z "$changed_files" ] && return 0
    # shellcheck disable=SC2086
    local dupes
    dupes="$(grep -hE '^[[:space:]]*(def |function |func )[A-Za-z_][A-Za-z_0-9]*' $changed_files 2>/dev/null \
        | sed 's/^[[:space:]]*//' \
        | grep -oE '(def |function |func )[A-Za-z_][A-Za-z_0-9]*' \
        | awk '{print $2}' \
        | sort | uniq -d)" || true
    # Issue #1245: exempt conventional method names that legitimately recur
    # across distinct classes/files (e.g. every unittest.TestCase defines its
    # own setUp/tearDown). Flagging these is a false positive; a real accidental
    # dup of a domain function name still flags.
    local _DUP_NAME_EXEMPT=" setUp tearDown setUpClass tearDownClass asyncSetUp asyncTearDown __init__ __enter__ __exit__ main "
    if [ -n "$dupes" ]; then
        echo "$dupes" | while IFS= read -r dupe_name; do
            [ -z "$dupe_name" ] && continue
            case "$_DUP_NAME_EXEMPT" in
                *" $dupe_name "*) continue ;;
            esac
            emit_capped "COMPLEXITY" "-" "-" "duplicate function name '${dupe_name}' across changed files — reuse or rename to avoid confusion"
        done
    fi
}


# ── ripgrep-backed reuse detectors (issue #1439) ───────────────────────────────
# Sourced from scripts/lib/ so this file can shrink under the size ratchet. Must
# come after the emit_*/is_line_allowed helpers the module calls.
#
# ${0%/*} rather than $(dirname "$0"): the reuse-lens tests deliberately run this
# script on a near-empty PATH to exercise the rg fail-open, and a subshell calling
# dirname exits 127 there.
_LINT_SELF_DIR="${0%/*}"
if [ "$_LINT_SELF_DIR" = "$0" ]; then _LINT_SELF_DIR="."; fi
if [ -f "$_LINT_SELF_DIR/lib/lint-reuse-lens.sh" ]; then
    . "$_LINT_SELF_DIR/lib/lint-reuse-lens.sh"
else
    # A missing module means a broken install (shipped via install.sh runtime_libs).
    # Stub so the gate still runs, but SAY so — silently skipping the lens is the
    # exact failure this module exists to remove.
    detect_reinvent_repo_util() { :; }
    detect_new_abstraction_single_caller() {
        emit_info REUSE_LENS_DISABLED "-" "-" \
            'lib/lint-reuse-lens.sh not found next to this script; the rg-backed reuse detectors are inert. Re-run install.sh --update.'
    }
fi

# ── §3.x NEW_DEP_UNJUSTIFIED detector ────────────────────────────────────────
# Dependency added to a manifest without a why: justification in the same hunk.

_ndj_dep_pattern() {
    case "$1" in
        *requirements.txt) printf '^[A-Za-z][A-Za-z0-9_.-]' ;;
        *package.json)     printf '"[A-Za-z@][^"]+": "[0-9^~*>=]' ;;
        *go.mod)           printf '^[[:space:]]*(require[[:space:]]+)?[A-Za-z][^ ]* v[0-9]' ;;
        *Cargo.toml)       printf '^[a-z_-][a-z0-9_-]* *= *"' ;;
        *pyproject.toml)   printf '[A-Za-z][A-Za-z0-9_-]*[[:space:]]*[>=<!]' ;;
        *Gemfile)          printf "^gem ['\"]" ;;
        *)                 printf '' ;;
    esac
}

detect_new_dep_unjustified() {
    local _ndj_tmp
    _ndj_tmp="$(mktemp -t lint-dep-unjust.XXXXXX)"

    while IFS= read -r diff_file; do
        [ -z "$diff_file" ] && continue
        local _ndj_pat
        _ndj_pat="$(_ndj_dep_pattern "$diff_file")"
        if [ -z "$_ndj_pat" ]; then
            continue
        fi

        # Parse this file's diff hunk-by-hunk.
        # Output "LINENO:CONTENT" for dep-add lines in hunks that lack a why: marker.
        awk -v tgt="$diff_file" -v pat="$_ndj_pat" '
            /^diff --git / {
                in_file = ($0 ~ " b/" tgt "$")
                next
            }
            !in_file { next }
            /^@@ / {
                if (n > 0 && !has_why) {
                    for (i = 0; i < n; i++) print lnos[i] ":" lines[i]
                }
                n = 0; has_why = 0
                hdr = $0; sub(/.*\+/, "", hdr); sub(/[^0-9].*/, "", hdr)
                cur = hdr + 0; next
            }
            /^\+\+\+ |^--- / { next }
            /^ / {
                cur++
                if ($0 ~ /[Ww]hy:/) has_why = 1
                next
            }
            /^\+/ {
                cur++
                content = substr($0, 2)
                if (content ~ /[Ww]hy:/) { has_why = 1; next }
                if (match(content, pat)) {
                    lines[n] = content; lnos[n] = cur; n++
                }
            }
            END {
                if (n > 0 && !has_why) {
                    for (i = 0; i < n; i++) print lnos[i] ":" lines[i]
                }
            }
        ' "$TMP_DIFF" > "$_ndj_tmp"

        while IFS= read -r _ndj_entry; do
            [ -z "$_ndj_entry" ] && continue
            local _ndj_lno _ndj_rest
            _ndj_lno="$(printf '%s' "$_ndj_entry" | cut -d: -f1)"
            _ndj_rest="$(printf '%s' "$_ndj_entry" | cut -d: -f2-)"
            if ! is_line_allowed "NEW_DEP_UNJUSTIFIED" "$diff_file" "$_ndj_lno"; then
                emit_capped "NEW_DEP_UNJUSTIFIED" "$diff_file" "$_ndj_lno" \
                    "dependency added without 'why:' justification: ${_ndj_rest}"
            fi
        done < "$_ndj_tmp"
    done <<EOF
$(get_diff_files)
EOF

    rm -f "$_ndj_tmp"
}

# ── directives output mode ────────────────────────────────────────────────────
# Maps each RULE_ID to a short imperative directive line.

rule_directive() {
    local rule_id="$1"
    case "$rule_id" in
        PR_SIZE)        printf 'Freeze the completed capped slice and move unmet acceptance criteria to ordered continuation issues; never push or merge this oversized diff.' ;;
        OUT_OF_SCOPE)    printf 'Restrict diff to files listed in the issue ## Implementation outline; revert or amend the issue body for any extra files.' ;;
        MISSING_TEST)    printf 'Add a test under tests/<tier>/ for the missing required test type before re-pushing.' ;;
        COMPLEXITY)      printf 'Split functions >50 LOC, files >500 LOC, or nesting >4 — no copy-paste branches.' ;;
        SECURITY)        printf 'Remove the flagged pattern: never hardcode secrets, never bypass git hooks or use destructive resets, validate all inputs.' ;;
        TODO_LEFT)       printf 'Remove deferred-work markers from non-test code; file a follow-up issue for genuinely deferred work.' ;;
        MOCK_DB)         printf 'Remove DB mock/stub; use the real database per AGENTS.md ## Engineering standards.' ;;
        DOC_OUT_OF_SYNC) printf 'Update the doc file(s) covering the changed public surface (CLI flag/env var/export) in this same PR.' ;;
        HALLUCINATED_API) printf 'Verify the flagged symbol exists in the repo or dependency manifests before using it.' ;;
        DUPLICATE_CODE)  printf 'Reuse the existing helper instead of re-implementing the same logic.' ;;
        INVENTED_CONFIG) printf 'Remove the invented flag/env/key, or amend the issue body to introduce it as in-scope.' ;;
        VACUOUS_GREP_INVERSE_OR_TRUE) printf 'Replace `grep -qv "X" || true` with `! grep -q "X"` — the current form always exits 0.' ;;
        VACUOUS_OR_TRUE) printf 'Remove `|| true` from the assertion line so failures propagate correctly.' ;;
        VACUOUS_TAUTOLOGY) printf 'Replace the tautological assertion with one that checks real output from the code under test.' ;;
        VACUOUS_AC_STUB) printf 'Replace the auto-stub skip with a real assertion that exercises the acceptance criterion.' ;;
        VACUOUS_EMPTY_TEST) printf 'Add at least one assertion to the empty test body.' ;;
        VACUOUS_NO_ASSERT) printf 'Add an assert/expect/run+grep call to the test so it can actually fail.' ;;
        ASSERTION_DENSITY) printf 'Add at least one assert/expect/run/grep call to each test block — zero-assertion tests cannot catch regressions.' ;;
        REINVENT_REPO_UTIL) printf 'Reuse the existing helper found in scripts/ instead of re-implementing the same function.' ;;
        NEW_DEP_UNJUSTIFIED) printf "Add a '# why: <reason>' comment in the same diff hunk justifying this new dependency." ;;
        NEW_ABSTRACTION_SINGLE_CALLER) printf 'Inline this abstraction — with only one caller, the named wrapper adds indirection without value.' ;;
        *)               printf 'Fix the flagged %s violation before re-pushing.' "$rule_id" ;;
    esac
}

# ── main ──────────────────────────────────────────────────────────────────────

if [ "$DIRECTIVES" -eq 1 ]; then
    # Capture findings to a temp file, then reformat as directives
    TMP_FINDINGS="$(mktemp -t lint-impl-findings.XXXXXX)"
    trap 'rm -f "$TMP_DIFF" "$TMP_ISSUE" "$TMP_CONTRACT_OUT" "$TMP_CONTRACT_ERR" "$TMP_FINDINGS"' EXIT INT TERM

    # Run detectors with stdout going to TMP_FINDINGS
    {
        detect_pr_size
        detect_implementation_contract
        detect_complexity
        check_file_loc
        check_function_loc
        check_cyclomatic
        check_duplicate_names
        detect_security
        detect_todo_left
        detect_mock_db
        detect_doc_out_of_sync
        if [ "$VACUOUS_ASSERTIONS" -eq 1 ]; then
            detect_vacuous_assertions
        fi
        if [ "$ASSERTION_DENSITY" -eq 1 ]; then
            detect_assertion_density
        fi
        # Reuse-interrogation triage (issue #1439) is part of the reuse lens and
        # must be inert unless AUTOSPEC_REUSE_LENS=1 — otherwise the lens fires
        # while disarmed (flag-OFF byte-identical AC; spec Error handling §).
        if [ "${AUTOSPEC_REUSE_LENS:-}" = "1" ]; then
            detect_reinvent_repo_util
            detect_new_dep_unjustified
            detect_new_abstraction_single_caller
        fi
    } > "$TMP_FINDINGS" 2>&1

    # Reformat each finding as a directive line
    while IFS= read -r finding; do
        # Extract RULE_ID from "RULE_ID:path:line: desc" format
        rule_id="$(printf '%s' "$finding" | cut -d: -f1)"
        # Skip INFO lines
        if [ "$rule_id" = "INFO" ]; then
            continue
        fi
        if [ "$rule_id" = "ERROR" ]; then
            rule_id="$(printf '%s' "$finding" | cut -d: -f2)"
        fi
        directive="$(rule_directive "$rule_id")"
        printf 'Fix %s: %s\n' "$rule_id" "$directive"
    done < "$TMP_FINDINGS"
else
    detect_pr_size
    detect_implementation_contract
    detect_complexity
    check_file_loc
    check_function_loc
    check_cyclomatic
    check_duplicate_names
    detect_security
    detect_todo_left
    detect_mock_db
    detect_doc_out_of_sync
    if [ "$VACUOUS_ASSERTIONS" -eq 1 ]; then
        detect_vacuous_assertions
    fi
    if [ "$ASSERTION_DENSITY" -eq 1 ]; then
        detect_assertion_density
    fi
    # Reuse-interrogation triage (issue #1439): inert unless the lens is armed.
    if [ "${AUTOSPEC_REUSE_LENS:-}" = "1" ]; then
        detect_reinvent_repo_util
        detect_new_dep_unjustified
        detect_new_abstraction_single_caller
    fi
fi

# Exit with min(FINDINGS_COUNT, FINDINGS_EXIT_CAP)
if [ "$FINDINGS_COUNT" -eq 0 ]; then
    exit 0
elif [ "$FINDINGS_COUNT" -gt "$FINDINGS_EXIT_CAP" ]; then
    exit "$FINDINGS_EXIT_CAP"
else
    exit "$FINDINGS_COUNT"
fi
