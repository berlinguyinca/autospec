#!/usr/bin/env bats
# tests/unit/test_validate_extension.bats — exercise the new check functions
# added to scripts/validate.sh (presence checks for skills/autospec-listen,
# examples/, and governance headings in AGENTS.md / README.md / SKILLS.md).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    VALIDATE="$REPO_ROOT/scripts/validate.sh"
    # Per-test scratch root mirroring the repo layout for hermetic checks.
    SCRATCH="$(mktemp -d)"
    export SCRATCH
}

teardown() {
    if [ -n "${SCRATCH:-}" ] && [ -d "$SCRATCH" ]; then
        rm -rf "$SCRATCH"
    fi
}

# ---- baseline ------------------------------------------------------------

@test "validate.sh: passes against the real repo as-is" {
    run bash "$VALIDATE"
    [ "$status" -eq 0 ]
}

@test "validate.sh: bash -n syntax check passes" {
    run bash -n "$VALIDATE"
    [ "$status" -eq 0 ]
}

# ---- new check functions exist as named functions ------------------------

@test "validate.sh: defines check_autospec_listen_files" {
    run grep -q '^check_autospec_listen_files()' "$VALIDATE"
    [ "$status" -eq 0 ]
}

@test "validate.sh: defines check_examples_dir" {
    run grep -q '^check_examples_dir()' "$VALIDATE"
    [ "$status" -eq 0 ]
}

@test "validate.sh: defines check_governance_headings" {
    run grep -q '^check_governance_headings()' "$VALIDATE"
    [ "$status" -eq 0 ]
}

# ---- removing a required listener file makes validate fail ---------------

# Helper: clone the repo working set into a per-test scratch, run the
# scratch copy of validate.sh and capture its exit code.
_run_scratch_validate() {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    (cd "$SCRATCH" && bash scripts/validate.sh)
}

@test "validate.sh: fails when skills/autospec-listen/SKILL.md is missing" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    rm -f "$SCRATCH/skills/autospec-listen/SKILL.md"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'skills/autospec-listen/SKILL.md'
}

@test "validate.sh: fails when skills/autospec-listen/install.sh is missing" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    rm -f "$SCRATCH/skills/autospec-listen/install.sh"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    # The pre-existing per-skill check reports as
    # "autospec-listen: missing required file install.sh"; the new check
    # reports as "skills/autospec-listen/install.sh: required file missing".
    # Either is acceptable.
    echo "$output" | grep -E 'install\.sh' >/dev/null
    echo "$output" | grep -F 'autospec-listen' >/dev/null
}

@test "validate.sh: fails when skills/autospec-listen/uninstall.sh is missing" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    rm -f "$SCRATCH/skills/autospec-listen/uninstall.sh"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -E 'uninstall\.sh' >/dev/null
    echo "$output" | grep -F 'autospec-listen' >/dev/null
}

@test "validate.sh: fails when skills/autospec-listen/opencode/agent.md is missing" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    rm -f "$SCRATCH/skills/autospec-listen/opencode/agent.md"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'skills/autospec-listen/opencode/agent.md'
}

@test "validate.sh: fails when skills/autospec-listen/codex/prompt.md is missing" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    rm -f "$SCRATCH/skills/autospec-listen/codex/prompt.md"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'skills/autospec-listen/codex/prompt.md'
}

@test "validate.sh: fails when references/trigger-keywords.md is missing" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    rm -f "$SCRATCH/skills/autospec-listen/references/trigger-keywords.md"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'references/trigger-keywords.md'
}

@test "validate.sh: fails when skills/autospec-listen/README.md is missing" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    rm -f "$SCRATCH/skills/autospec-listen/README.md"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'README.md' >/dev/null
    echo "$output" | grep -F 'autospec-listen' >/dev/null
}

# ---- removing a required examples file makes validate fail --------------

@test "validate.sh: fails when examples/model-profiles.yml is missing" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    rm -f "$SCRATCH/examples/model-profiles.yml"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'examples/model-profiles.yml'
}

@test "validate.sh: fails when examples/project-map.yml is missing" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    rm -f "$SCRATCH/examples/project-map.yml"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'examples/project-map.yml'
}

@test "validate.sh: fails when examples/README.md is missing" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    rm -f "$SCRATCH/examples/README.md"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'examples/README.md'
}

# ---- governance headings ------------------------------------------------

@test "validate.sh: fails when AGENTS.md is missing '## Anti-loop guardrails' heading" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    # Rewrite AGENTS.md without the heading.
    grep -v '^## Anti-loop guardrails' "$REPO_ROOT/AGENTS.md" > "$SCRATCH/AGENTS.md"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'Anti-loop guardrails'
}

@test "validate.sh: fails when AGENTS.md is missing '## Listener-filed issues lifecycle' heading" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    grep -v '^## Listener-filed issues lifecycle' "$REPO_ROOT/AGENTS.md" > "$SCRATCH/AGENTS.md"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'Listener-filed issues lifecycle'
}

@test "validate.sh: fails when README.md does not mention autospec-listen" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    grep -v 'autospec-listen' "$REPO_ROOT/README.md" > "$SCRATCH/README.md"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'README.md'
}

@test "validate.sh: fails when SKILLS.md does not mention autospec-listen" {
    cp -R "$REPO_ROOT"/. "$SCRATCH"/
    rm -rf "$SCRATCH/.git"
    grep -v 'autospec-listen' "$REPO_ROOT/SKILLS.md" > "$SCRATCH/SKILLS.md"
    run bash -c "cd '$SCRATCH' && bash scripts/validate.sh 2>&1"
    [ "$status" -ne 0 ]
    echo "$output" | grep -F 'SKILLS.md'
}
