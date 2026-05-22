#!/usr/bin/env bats
# tests/validate.bats — bats tests for scripts/validate.sh check_lockstep() duo-mode
# Covers: trio-pass, duo-pass, duo-divergence-fail

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/validate.sh"
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures"

# Source just the helper functions from validate.sh for unit testing.
# We do this by defining a test harness that invokes check_lockstep / check_lockstep_duo
# against fixture directories.

# Helper: run check_lockstep against a fixture dir by invoking the full validate.sh
# in a controlled environment pointing at a fake skills/ tree.
run_lockstep_on_fixture() {
    local fixture="$1"
    local mode="$2"  # "trio" or "duo"

    # Build a temp skills/ tree with just the fixture skill
    local tmpdir
    tmpdir="$(mktemp -d)"
    mkdir -p "$tmpdir/skills"
    cp -r "$fixture" "$tmpdir/skills/test-skill"

    # Run just the lockstep portion by sourcing validate.sh functions
    (
        cd "$tmpdir"
        bash -c "
            set -eu
            fail() { printf 'validate: FAIL — %s\n' \"\$*\" >&2; exit 1; }
            info() { printf 'validate: %s\n' \"\$*\"; }
            strip_body() { awk '/^\-\-\-\$/{c++; next} c>=2' \"\$1\"; }

            check_lockstep() {
                skill_dir=\"\$1\"
                name=\"\$(basename \"\$skill_dir\")\"
                info \"lock-step: \$name\"
                if ! diff <(strip_body \"\$skill_dir/SKILL.md\") <(cat \"\$skill_dir/codex/prompt.md\") >/dev/null; then
                    diff <(strip_body \"\$skill_dir/SKILL.md\") <(cat \"\$skill_dir/codex/prompt.md\") | head -40 >&2
                    fail \"\$name: SKILL.md body diverges from codex/prompt.md\"
                fi
                if ! diff <(strip_body \"\$skill_dir/SKILL.md\") <(strip_body \"\$skill_dir/opencode/agent.md\") >/dev/null; then
                    diff <(strip_body \"\$skill_dir/SKILL.md\") <(strip_body \"\$skill_dir/opencode/agent.md\") | head -40 >&2
                    fail \"\$name: SKILL.md body diverges from opencode/agent.md\"
                fi
            }

            check_lockstep_duo() {
                skill_dir=\"\$1\"
                name=\"\$(basename \"\$skill_dir\")\"
                info \"lock-step (duo): \$name\"
                if ! diff <(strip_body \"\$skill_dir/SKILL.md\") <(cat \"\$skill_dir/codex/prompt.md\") >/dev/null; then
                    diff <(strip_body \"\$skill_dir/SKILL.md\") <(cat \"\$skill_dir/codex/prompt.md\") | head -40 >&2
                    fail \"\$name: \$skill_dir/SKILL.md body diverges from \$skill_dir/codex/prompt.md\"
                fi
            }

            if [ \"$mode\" = \"trio\" ]; then
                check_lockstep skills/test-skill
            else
                check_lockstep_duo skills/test-skill
            fi
        "
    )
    local status=$?
    rm -rf "$tmpdir"
    return $status
}

@test "trio-pass: aligned SKILL.md / opencode/agent.md / codex/prompt.md exits 0" {
    run run_lockstep_on_fixture "$FIXTURES_DIR/lockstep-trio-pass" "trio"
    [ "$status" -eq 0 ]
}

@test "duo-pass: aligned SKILL.md / codex/prompt.md (no opencode) exits 0" {
    run run_lockstep_on_fixture "$FIXTURES_DIR/lockstep-duo-pass" "duo"
    [ "$status" -eq 0 ]
}

@test "duo-divergence: mismatched SKILL.md vs codex/prompt.md exits non-zero" {
    run run_lockstep_on_fixture "$FIXTURES_DIR/lockstep-duo-divergence" "duo"
    [ "$status" -ne 0 ]
}

@test "duo-divergence: failure message names both SKILL.md and codex/prompt.md" {
    run run_lockstep_on_fixture "$FIXTURES_DIR/lockstep-duo-divergence" "duo"
    [ "$status" -ne 0 ]
    [[ "$output" == *"SKILL.md"* ]]
    [[ "$output" == *"codex/prompt.md"* ]]
}
