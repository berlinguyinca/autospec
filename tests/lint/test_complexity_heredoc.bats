#!/usr/bin/env bats
# tests/lint/test_complexity_heredoc.bats — heredoc-aware COMPLEXITY detector (issue #656).
#
# scripts/lint-implementation.sh's COMPLEXITY detector measures nesting depth
# from physical indentation. Lines inside heredocs (`<<EOF`, `<<'EOF'`,
# `<<-EOF`, etc.) are string literals, not code — but the legacy detector
# counted their indentation as code nesting. This suite locks in the fix.
#
# Every assertion accepts an optional INFO: prefix, because this suite is about whether
# the detector SEES the nesting, not about whether seeing it blocks a commit — COMPLEXITY
# is advisory unless AUTOSPEC_COMPLEXITY_ENFORCE=1. Anchoring on the blocking prefix would
# make the four "emits no COMPLEXITY" cases pass on any output at all.

setup() {
    REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
    LINT="${REPO_ROOT}/scripts/lint-implementation.sh"
    TMPDIR_BATS="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMPDIR_BATS"
}

# Build a synthetic git diff from a new-file payload.
# Usage: make_diff <relative-path> <content-file>
make_diff() {
    local rel="$1" src="$2" out="$3"
    local nlines
    nlines="$(wc -l < "$src" | tr -d ' ')"
    {
        printf 'diff --git a/%s b/%s\n' "$rel" "$rel"
        printf 'new file mode 100644\n'
        printf '%s\n' '--- /dev/null'
        printf '+++ b/%s\n' "$rel"
        printf '@@ -0,0 +1,%s @@\n' "$nlines"
        sed 's/^/+/' "$src"
    } > "$out"
}

@test "heredoc with single-quoted marker and deeply-indented content emits no COMPLEXITY" {
    src="$TMPDIR_BATS/heredoc_single.sh"
    cat > "$src" <<'PAYLOAD'
#!/usr/bin/env bash
emit() {
    cat <<'EOF'
                            line indented 28 spaces
                                    line indented 36 spaces
                                            line indented 44 spaces
                                                    line indented 52 spaces
EOF
}
PAYLOAD
    diff="$TMPDIR_BATS/d.diff"
    make_diff "scripts/heredoc_single.sh" "$src" "$diff"
    run bash "$LINT" --diff-file "$diff"
    [ "$(printf '%s\n' "$output" | grep -cE '^(INFO:)?COMPLEXITY:')" -eq 0 ]
}

@test "heredoc with unquoted marker (variable-expanding) is also skipped" {
    src="$TMPDIR_BATS/heredoc_unq.sh"
    cat > "$src" <<'PAYLOAD'
#!/usr/bin/env bash
greet() {
    cat <<EOF
                            depth proxy 7
                                    depth proxy 9
                                            depth proxy 11
                                                    depth proxy 13
EOF
}
PAYLOAD
    diff="$TMPDIR_BATS/d.diff"
    make_diff "scripts/heredoc_unq.sh" "$src" "$diff"
    run bash "$LINT" --diff-file "$diff"
    [ "$(printf '%s\n' "$output" | grep -cE '^(INFO:)?COMPLEXITY:')" -eq 0 ]
}

@test "heredoc with tab-stripped form (<<-EOF) closes on tab-indented marker" {
    src="$TMPDIR_BATS/heredoc_dash.sh"
    # POSIX/bash strips ONLY leading tabs from <<- body/closer; build the
    # fixture with real tab characters so the closer is valid bash.
    {
        printf '%s\n' '#!/usr/bin/env bash'
        printf '%s\n' 'emit() {'
        printf '%s\n' '    if true; then'
        printf '\t%s\n' 'cat <<-EOF'
        printf '%s\n' '                            indented body line A'
        printf '%s\n' '                                    indented body line B'
        printf '%s\n' '                                            indented body line C'
        printf '\t%s\n' 'EOF'
        printf '%s\n' '    fi'
        printf '%s\n' '}'
    } > "$src"
    diff="$TMPDIR_BATS/d.diff"
    make_diff "scripts/heredoc_dash.sh" "$src" "$diff"
    run bash "$LINT" --diff-file "$diff"
    [ "$(printf '%s\n' "$output" | grep -cE '^(INFO:)?COMPLEXITY:')" -eq 0 ]
}

@test "real bash control flow nested 5+ levels still emits COMPLEXITY" {
    src="$TMPDIR_BATS/real_nest.sh"
    cat > "$src" <<'PAYLOAD'
#!/usr/bin/env bash
deep() {
    if true; then
        for i in 1 2 3; do
            while read -r line; do
                case "$line" in
                    a)
                        if [ -n "$line" ]; then
                            printf 'deep %s\n' "$line"
                        fi
                        ;;
                esac
            done
        done
    fi
}
PAYLOAD
    diff="$TMPDIR_BATS/d.diff"
    make_diff "scripts/real_nest.sh" "$src" "$diff"
    run bash "$LINT" --diff-file "$diff"
    [ "$(printf '%s\n' "$output" | grep -cE '^(INFO:)?COMPLEXITY:.*nesting depth')" -ge 1 ]
}

@test "heredoc inside function with only shallow real control flow emits no COMPLEXITY" {
    src="$TMPDIR_BATS/shallow_heredoc.sh"
    cat > "$src" <<'PAYLOAD'
#!/usr/bin/env bash
render() {
    local who="$1"
    cat <<'EOF'
                            template line one indented deep
                                    template line two indented deeper
                                            template line three indented deepest
                                                    template line four indented absurd
EOF
    printf 'hello %s\n' "$who"
}
PAYLOAD
    diff="$TMPDIR_BATS/d.diff"
    make_diff "scripts/shallow_heredoc.sh" "$src" "$diff"
    run bash "$LINT" --diff-file "$diff"
    [ "$(printf '%s\n' "$output" | grep -cE '^(INFO:)?COMPLEXITY:')" -eq 0 ]
}

@test "real qa-finding-filter.sh and qa-verify-finding.sh produce zero COMPLEXITY findings" {
    # Synthesize a diff of the entire current files (treated as added lines).
    diff="$TMPDIR_BATS/dogfood.diff"
    : > "$diff"
    for f in scripts/qa-finding-filter.sh scripts/qa-verify-finding.sh; do
        [ -f "${REPO_ROOT}/$f" ] || skip "missing $f"
        nlines="$(wc -l < "${REPO_ROOT}/$f" | tr -d ' ')"
        {
            printf 'diff --git a/%s b/%s\n' "$f" "$f"
            printf 'new file mode 100644\n'
            printf '%s\n' '--- /dev/null'
            printf '+++ b/%s\n' "$f"
            printf '@@ -0,0 +1,%s @@\n' "$nlines"
            sed 's/^/+/' "${REPO_ROOT}/$f"
        } >> "$diff"
    done
    run bash "$LINT" --diff-file "$diff"
    complexity_lines="$(printf '%s\n' "$output" | grep -E '^(INFO:)?COMPLEXITY:' || true)"
    [ -z "$complexity_lines" ] || {
        printf 'unexpected COMPLEXITY findings:\n%s\n' "$complexity_lines" >&2
        return 1
    }
}
