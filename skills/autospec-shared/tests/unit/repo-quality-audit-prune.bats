#!/usr/bin/env bats
# repo-quality-audit-prune.bats — discovery pruning for the shared repo quality audit.
#
# Split out of repo-quality-audit.bats: that file is past the size ceiling and the
# ratchet refuses to let it grow, so the nested-tree coverage below could not be
# added there.

if [ -z "${BATS_TEST_FILENAME:-}" ]; then
    exec bats "$0" "$@"
fi

load './repo-quality-audit-helpers'

setup() {
    setup_repo_quality_audit_fixture
}

teardown() {
    teardown_repo_quality_audit_fixture
}

@test "Rust probes prune dependency and generated trees" {
    printf '%s\n' 'Numeric invariants: money values must use Decimal.' > "$REPO/AGENTS.md"
    mkdir -p "$REPO/target" "$REPO/vendor" "$REPO/node_modules" "$REPO/dist"
    printf 'pub const PRICE: f64 = 1.0;\n' > "$REPO/src/included.rs"
    for excluded in target vendor node_modules dist; do
        printf 'pub const PRICE: f64 = 1.0;\n' > "$REPO/$excluded/generated.rs"
    done

    run bash "$AUDIT" --repo "$REPO" --json "$TEST_TMP/audit.json" --markdown "$TEST_TMP/audit.md"
    [ "$status" -eq 0 ]
    run jq -e '[.findings[] | select(.probe=="f64-numeric-invariant")] | length == 1' "$TEST_TMP/audit.json"
    [ "$status" -eq 0 ]
    run jq -e '.findings[] | select(.probe=="f64-numeric-invariant") | .file=="src/included.rs"' "$TEST_TMP/audit.json"
    [ "$status" -eq 0 ]
}

@test "Rust probes prune nested dependency and generated trees" {
    printf '%s\n' 'Numeric invariants: money values must use Decimal.' > "$REPO/AGENTS.md"
    printf 'pub const PRICE: f64 = 1.0;\n' > "$REPO/src/included.rs"

    # A Cargo workspace puts a target/ under each crate, and agent worktrees under
    # .claude/ are whole repo copies carrying their own. Root-anchored pruning saw
    # none of these and scanned build output as first-party source.
    for excluded in \
        "crates/inner/target" \
        "crates/inner/vendor" \
        "packages/web/node_modules" \
        ".claude/worktrees/agent-a/src"; do
        mkdir -p "$REPO/$excluded"
        printf 'pub const PRICE: f64 = 1.0;\n' > "$REPO/$excluded/generated.rs"
    done

    # A nested directory whose name a real source tree also uses must still be scanned:
    # pruning those by name would drop first-party code in other repos this audit runs on.
    mkdir -p "$REPO/src/build"
    printf 'pub const PRICE: f64 = 1.0;\n' > "$REPO/src/build/kept.rs"

    run bash "$AUDIT" --repo "$REPO" --json "$TEST_TMP/audit.json" --markdown "$TEST_TMP/audit.md"
    [ "$status" -eq 0 ]
    run jq -e '[.findings[] | select(.probe=="f64-numeric-invariant") | .file] | sort == ["src/build/kept.rs","src/included.rs"]' "$TEST_TMP/audit.json"
    [ "$status" -eq 0 ]
}
