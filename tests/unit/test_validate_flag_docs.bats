#!/usr/bin/env bats
# tests/unit/test_validate_flag_docs.bats — validates that used ~/.autospec/*.flag
# sentinels are recorded in docs/FLAGS.md.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    VALIDATE="$REPO_ROOT/scripts/validate.sh"
    SCRATCH="$(mktemp -d)"
}

teardown() {
    [ -n "${SCRATCH:-}" ] && rm -rf "$SCRATCH"
}

extract_flag_doc_func() {
    awk '
        $0 ~ /^check_flag_sentinel_docs\(\) \{/ {grab=1}
        grab {print}
        grab && /^\}/ {grab=0}
    ' "$VALIDATE"
}

run_flag_doc_check_in_tree() {
    local tree="$1"
    (
        cd "$tree"
        bash -c "
            set -u
            fail() { printf 'validate: FAIL — %s\n' \"\$*\" >&2; exit 1; }
            info() { printf 'validate: %s\n' \"\$*\"; }
            $(extract_flag_doc_func)
            check_flag_sentinel_docs
        "
    )
}

new_flag_doc_tree() {
    local t
    t="$(mktemp -d)"
    mkdir -p "$t/scripts" "$t/docs" "$t/tests/unit"
    cat > "$t/scripts/uses-flags.sh" <<'SH'
#!/usr/bin/env bash
set -eu
[ -f "$HOME/.autospec/documented.flag" ] && echo documented
[ -f "${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/missing-sentinel.flag" ] && echo missing
SH
    cat > "$t/tests/unit/fixture.txt" <<'SH'
# test-only sentinel mentions do not require docs
touch "$HOME/.autospec/test-only.flag"
SH
    cat > "$t/docs/FLAGS.md" <<'MD'
# autospec flag-file reference

| Flag file | Effect | Set / cleared by |
|---|---|---|
| `documented.flag` | Fixture documented sentinel. | test |
MD
    printf '%s\n' "$t"
}

@test "check_flag_sentinel_docs is wired into validate.sh main" {
    run grep -q '^check_flag_sentinel_docs()' "$VALIDATE"
    [ "$status" -eq 0 ]
    run grep -Eq '^[[:space:]]+check_flag_sentinel_docs$' "$VALIDATE"
    [ "$status" -eq 0 ]
}

@test "check_flag_sentinel_docs fails when a source sentinel is missing from docs/FLAGS.md" {
    TREE="$(new_flag_doc_tree)"

    run bash -c "$(declare -f extract_flag_doc_func); $(declare -f run_flag_doc_check_in_tree); VALIDATE='$VALIDATE'; run_flag_doc_check_in_tree '$TREE'"

    [ "$status" -ne 0 ]
    [[ "$output" == *"missing-sentinel.flag"* ]]
    [[ "$output" != *"test-only.flag"* ]]
}

@test "check_flag_sentinel_docs passes when every source sentinel is documented" {
    TREE="$(new_flag_doc_tree)"
    printf '| `missing-sentinel.flag` | Fixture missing sentinel. | test |\n' >> "$TREE/docs/FLAGS.md"

    run bash -c "$(declare -f extract_flag_doc_func); $(declare -f run_flag_doc_check_in_tree); VALIDATE='$VALIDATE'; run_flag_doc_check_in_tree '$TREE'"

    [ "$status" -eq 0 ]
}

@test "check_flag_sentinel_docs ignores ignored cache artifacts containing flag-like bytes" {
    TREE="$(new_flag_doc_tree)"
    printf '| `missing-sentinel.flag` | Fixture missing sentinel. | test |\n' >> "$TREE/docs/FLAGS.md"
    mkdir -p "$TREE/packages/pkg/__pycache__"
    printf 'compiled-cache ghost-cache.flag bytes\n' > "$TREE/packages/pkg/__pycache__/module.pyc"
    printf '__pycache__/\n' > "$TREE/.gitignore"
    (
        cd "$TREE"
        git init -q
        git add .gitignore docs scripts
    )

    run bash -c "$(declare -f extract_flag_doc_func); $(declare -f run_flag_doc_check_in_tree); VALIDATE='$VALIDATE'; run_flag_doc_check_in_tree '$TREE'"

    [ "$status" -eq 0 ]
    [[ "$output" != *"ghost-cache.flag"* ]]
}
