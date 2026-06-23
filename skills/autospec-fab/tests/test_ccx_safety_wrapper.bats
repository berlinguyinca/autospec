#!/usr/bin/env bats
# test_ccx_safety_wrapper.bats — thin bats wrapper around the Python unittest suite
# for the ccx→SAFETY_FACTOR result-contract wrapper (issue #1300). Invoked by
# validate.sh check_autospec_fab_contract which runs every fab tests/*.bats file.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)"
SCRIPTS_DIR="$REPO_ROOT/skills/autospec-fab/scripts"
WRAPPERS_DIR="$REPO_ROOT/skills/autospec-fab/docker/wrappers"

@test "ccx safety wrapper python unittest suite passes" {
    command -v python3 || skip "python3 not found"
    run env PYTHONPATH="$SCRIPTS_DIR:$WRAPPERS_DIR" \
        python3 -m unittest discover \
            -s "$REPO_ROOT/skills/autospec-fab/tests" \
            -p "test_ccx_safety_wrapper.py" \
            -v
    [ "$status" -eq 0 ]
}
