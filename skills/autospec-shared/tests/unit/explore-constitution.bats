#!/usr/bin/env bats
# explore-constitution.bats — explore-constitution.sh seed + deterministic filter.

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

@test "--filter drops empty-evidence (D1) and below-floor confidence (D2)" {
    run bash -c "printf '%s' '[{\"title\":\"a\",\"evidence\":\"real\",\"confidence\":0.8},{\"title\":\"b\",\"evidence\":\"\",\"confidence\":0.9},{\"title\":\"c\",\"evidence\":\"x\",\"confidence\":0.1}]' | bash \"$C\" --filter"
    [ "$status" -eq 0 ]
    # only 'a' survives
    echo "$output" | jq -e '. == ["a"] or ([.[].title] == ["a"])' >/dev/null || \
      [ "$(echo "$output" | jq -c '[.[].title]')" = '["a"]' ]
}

@test "--filter respects --min-confidence override" {
    run bash -c "printf '%s' '[{\"title\":\"c\",\"evidence\":\"x\",\"confidence\":0.1}]' | bash \"$C\" --filter --min-confidence 0.05"
    [ "$status" -eq 0 ]
    [ "$(echo "$output" | jq -c '[.[].title]')" = '["c"]' ]
}

@test "--filter on whitespace-only evidence is treated as empty (dropped)" {
    run bash -c "printf '%s' '[{\"title\":\"w\",\"evidence\":\"   \",\"confidence\":0.9}]' | bash \"$C\" --filter"
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
