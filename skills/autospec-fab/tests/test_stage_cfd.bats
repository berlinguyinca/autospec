#!/usr/bin/env bats
# test_stage_cfd.bats — thin bats wrapper around the Python unittest suite.
# Invoked by validate.sh check_autospec_fab_contract which runs every
# skills/autospec-fab/tests/*.bats file.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)"
SCRIPTS_DIR="$REPO_ROOT/skills/autospec-fab/scripts"

@test "stage_cfd python unittest suite passes" {
    command -v python3 || skip "python3 not found"
    run env PYTHONPATH="$SCRIPTS_DIR" \
        python3 -m unittest discover \
            -s "$REPO_ROOT/skills/autospec-fab/tests" \
            -p "test_stage_cfd.py" \
            -v
    [ "$status" -eq 0 ]
}
