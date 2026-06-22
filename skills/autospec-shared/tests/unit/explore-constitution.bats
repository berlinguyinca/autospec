#!/usr/bin/env bats
# explore-constitution.bats — explore-constitution.sh seed + deterministic filter.

# `run --separate-stderr` (used by the --filter JSON tests, so the stdout JSON
# isn't polluted by the script's stderr accounting line) is a bats >=1.5.0
# feature; declaring the minimum silences the BW02 warning. The repo ships
# bats-core 1.13.0, so this is satisfied.
bats_require_minimum_version 1.5.0

setup() {
    SCRIPT_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")/../.." && pwd)"
    C="${SCRIPT_DIR}/scripts/explore-constitution.sh"
    TMP="$(mktemp -d /tmp/autospec-constitution-XXXXXX)"
    BASH_BIN="$(command -v bash)"
}
teardown() { rm -rf "$TMP"; }

@test "--ensure seeds a constitution when absent (idempotent)" {
    out="$TMP/c.md"
    run env AUTOSPEC_EXPLORE_CONSTITUTION="$out" bash "$C" --ensure
    [ "$status" -eq 0 ]
    [ -f "$out" ]
    grep -q '^# Explore Proposal Constitution' "$out"
    # idempotent: second call does not error and does not duplicate
    printf 'EDITED\n' >> "$out"
    run env AUTOSPEC_EXPLORE_CONSTITUTION="$out" bash "$C" --ensure
    [ "$status" -eq 0 ]
    grep -q 'EDITED' "$out"   # not overwritten
}

@test "--show prints the constitution (seeding first if absent)" {
    out="$TMP/c.md"
    run env AUTOSPEC_EXPLORE_CONSTITUTION="$out" bash "$C" --show
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q 'D1 Evidence'
    printf '%s' "$output" | grep -q 'J4 Safety'
}

# NOTE: --filter writes its human-readable accounting line ("kept=N dropped=M
# ...") to STDERR and the filtered JSON array to STDOUT (explore-constitution.sh
# lines 106-107). bats `run` merges stderr into $output by default, which
# corrupts the JSON these tests pipe to jq. Use `--separate-stderr` so $output
# is the pure stdout JSON ($stderr holds the accounting line). The empty-input
# test below stays on the merged form because that path emits no stderr line.
@test "--filter drops empty-evidence (D1) and below-floor confidence (D2)" {
    run --separate-stderr bash -c "printf '%s' '[{\"title\":\"a\",\"evidence\":\"real\",\"confidence\":0.8},{\"title\":\"b\",\"evidence\":\"\",\"confidence\":0.9},{\"title\":\"c\",\"evidence\":\"x\",\"confidence\":0.1}]' | bash \"$C\" --filter"
    [ "$status" -eq 0 ]
    # only 'a' survives
    echo "$output" | jq -e '. == ["a"] or ([.[].title] == ["a"])' >/dev/null || \
      [ "$(echo "$output" | jq -c '[.[].title]')" = '["a"]' ]
}

@test "--filter respects --min-confidence override" {
    run --separate-stderr bash -c "printf '%s' '[{\"title\":\"c\",\"evidence\":\"x\",\"confidence\":0.1}]' | bash \"$C\" --filter --min-confidence 0.05"
    [ "$status" -eq 0 ]
    [ "$(echo "$output" | jq -c '[.[].title]')" = '["c"]' ]
}

@test "--filter on whitespace-only evidence is treated as empty (dropped)" {
    run --separate-stderr bash -c "printf '%s' '[{\"title\":\"w\",\"evidence\":\"   \",\"confidence\":0.9}]' | bash \"$C\" --filter"
    [ "$status" -eq 0 ]
    [ "$(echo "$output" | jq -c 'length')" = "0" ]
}

@test "--filter empty input -> []" {
    run bash -c "printf '' | bash \"$C\" --filter"
    [ "$status" -eq 0 ]
    [ "$output" = "[]" ]
}

@test "--filter jq missing -> exit 2 (fail-closed)" {
    mkdir -p "$TMP/empty"
    run bash -c "printf '%s' '[]' | env PATH=\"$TMP/empty\" \"$BASH_BIN\" \"$C\" --filter"
    [ "$status" -eq 2 ]
}

@test "no subcommand -> usage error exit 2" {
    run bash "$C"
    [ "$status" -eq 2 ]
}
