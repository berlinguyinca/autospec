#!/usr/bin/env bats
# test_release_gate.bats — thin bats wrapper around the Python unittest suite.
# Invoked by the validate.sh fab gate (check_autospec_fab_contract) which runs
# every skills/autospec-fab/tests/*.bats file.

# Resolve the repo root from this file's location (tests/ → skill/ → repo root)
REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)"
SCRIPTS_DIR="$REPO_ROOT/skills/autospec-fab/scripts"

@test "release_gate engine python unittest suite passes" {
    command -v python3 || skip "python3 not found"
    run env PYTHONPATH="$SCRIPTS_DIR" \
        python3 -m unittest discover \
            -s "$REPO_ROOT/skills/autospec-fab/tests" \
            -p "test_release_gate.py" \
            -v
    [ "$status" -eq 0 ]
}
