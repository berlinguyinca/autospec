#!/usr/bin/env bats
# tests/cli-reference-coverage.bats — every top-level `autospec` command is
# documented in docs/cli-reference.md, or named in the shrink-only gap baseline
# below (#3973).
#
# The gap baseline exists so the ratchet bites new commands at the PR that adds
# them: adding a COMMANDS entry without a doc row fails, and documenting a
# baselined command without removing it from the baseline also fails.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    MOD="$REPO_ROOT/crates/autospec-cli/src/commands/mod.rs"
    CLI_DOCS="$REPO_ROOT/docs/cli-reference.md"
    INVARIANTS="$REPO_ROOT/docs/invariants.md"
}

# commands_from_registry — the command names in the COMMANDS table, one per line.
commands_from_registry() {
    awk '/^const COMMANDS: /{f=1;next} f&&/^\];/{f=0} f' "$MOD" |
        grep -oE '^[[:space:]]*"?[a-z][a-z0-9-]*",|^[[:space:]]*\("[a-z][a-z0-9-]*",' |
        tr -d '("' |
        sed -e 's/^[[:space:]]*//' -e 's/[,[:space:]]*$//' | sort -u
}

# Documented but not yet removed from the baseline. Kept empty on purpose:
# shrink-only — never add an entry to it, delete the doc row instead.
DOC_GAP_BASELINE="lint
parent"

@test "command registry parses and lists the new observe command" {
    names="$(commands_from_registry)"
    [ -n "$names" ]
    echo "$names" | grep -qx "observe"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
    return 0
}

# coverage_findings — command names in the registry that are neither documented
# in $CLI_DOCS nor named in $DOC_GAP_BASELINE, one per line.
coverage_findings() {
    for cmd in $(commands_from_registry); do
        grep -q "\`autospec $cmd" "$CLI_DOCS" && continue
        echo "$DOC_GAP_BASELINE" | grep -qx "$cmd" && continue
        printf '%s\n' "$cmd"
    done
}

@test "every registered command is documented or baselined" {
    run coverage_findings
    [ "$status" -eq 0 ]
    [ -z "$output" ] || {
        echo "undocumented commands (add a row to docs/cli-reference.md): $output"
        false
    }
}

@test "an undocumented new command is a finding" {
    WORK="$(mktemp -d -t cli-ref-coverage.XXXXXX)"
    sed 's/^    ("init",/    ("brand-new-cmd",/' "$MOD" > "$WORK/mod.rs"
    MOD="$WORK/mod.rs"
    run coverage_findings
    [ "$output" = "brand-new-cmd" ]
}

@test "gap baseline is shrink-only: no baselined command is already documented" {
    stale=""
    for cmd in $DOC_GAP_BASELINE; do
        grep -q "\`autospec $cmd" "$CLI_DOCS" && stale="$stale $cmd"
    done
    [ -z "$stale" ] || {
        echo "remove from DOC_GAP_BASELINE (now documented):$stale"
        false
    }
}

@test "observe subcommands are documented with their flags" {
    for sub in step growth count gate; do
        grep -q "\`autospec observe $sub" "$CLI_DOCS"
    done
    grep "\`autospec observe step" "$CLI_DOCS" | grep -q -- "--interval"
    grep "\`autospec observe count" "$CLI_DOCS" | grep -q -- "--characterise"
    grep "\`autospec observe gate" "$CLI_DOCS" | grep -q -- "--terminal"
}

@test "the not-yet invariant names both observation implementation files" {
    row="$(grep 'issue #3973' "$INVARIANTS")"
    [ -n "$row" ]
    echo "$row" | grep -q "crates/autospec-core/src/observation.rs"
    echo "$row" | grep -q "crates/autospec-cli/src/commands/observe.rs"
}
