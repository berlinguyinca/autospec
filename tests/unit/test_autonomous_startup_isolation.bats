#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "autonomous dispatch disables skill self-update in the target checkout" {
    run rg -n -U 'AUTOSPEC_NO_SELF_UPDATE=1 \\\n+\s+(AUTOSPEC_RUN_ONLY_ISSUES="\$_prov_(operator|self)" \\\n+\s+)?bash -c "\$_(run|explore)_cmd' \
        "$REPO_ROOT/scripts/lib/autospec-loop.sh"
    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | wc -l)" -ge 3 ]
}

@test "autonomous launcher resolves an explicit repository directory" {
    run rg -n 'AUTOSPEC_REPO_DIR|--repo-dir|git rev-parse --show-toplevel' \
        "$REPO_ROOT/scripts/autospec-autonomous.sh"
    [ "$status" -eq 0 ]
    [[ "$output" == *"git rev-parse --show-toplevel"* ]]
}
