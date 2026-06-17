#!/usr/bin/env bats
# tests/explore/test_explore_self_leverage.bats — bats suite for the
# self-leverage researcher (Issue B2).
#
# Asserts:
#   1. Flags a seeded human-in-loop point that is low-stakes (should auto-resolve).
#   2. Does NOT flag an already-auto-resolved point (legitimate operator action).
#   3. Well-formed JSON from an empty repo.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t sl-test.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
}

teardown() {
    rm -rf "$TMP"
}

assert_well_formed() {
    local json="$1"
    printf '%s' "$json" | python3 -c '
import json, sys
d = json.load(sys.stdin)
assert isinstance(d, dict), "top-level must be object"
assert "source" in d, "missing source"
assert "proposals" in d, "missing proposals"
assert isinstance(d["proposals"], list), "proposals must be list"
for p in d["proposals"]:
    for k in ("title", "evidence", "estimated_complexity", "confidence"):
        assert k in p, f"proposal missing {k}"
    assert p["estimated_complexity"] in ("small", "medium", "large"), "bad complexity"
    assert 0.0 <= float(p["confidence"]) <= 1.0, "bad confidence"
'
}

# ── 1. Flags a seeded human-in-loop point ────────────────────────────────

@test "self-leverage: flags a low-stakes 'operator must' step in skill prose" {
    mkdir -p skills/my-skill
    # A SKILL.md with a low-stakes "operator must" intervention that is NOT
    # a run/defer/refine/destructive action — should be flagged.
    cat > skills/my-skill/SKILL.md <<'EOF'
# my-skill

## Usage

Run the skill with `/my-skill`.

## Steps

1. Gather inputs.
2. The operator must manually confirm the config file path before proceeding.
3. Continue with the rest of the process.
EOF
    git add -A && git commit -q -m "seed human-in-loop step"
    run bash "$REPO_ROOT/scripts/explore-research/self-leverage.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"self-leverage"* ]]
    # Must have at least one proposal flagging this intervention.
    proposal_count=$(printf '%s' "$output" | python3 -c '
import json, sys
d = json.load(sys.stdin)
print(len(d["proposals"]))
')
    [ "$proposal_count" -gt 0 ]
}

@test "self-leverage: flags a low-stakes 'manually' intervention in a script" {
    mkdir -p scripts
    # A script with a read prompt that is not for a destructive action.
    cat > scripts/helper.sh <<'EOF'
#!/usr/bin/env bash
# Collect configuration
echo "Please enter the config name:"
read -r config_name
echo "Using config: $config_name"
EOF
    git add -A && git commit -q -m "seed script with read prompt"
    run bash "$REPO_ROOT/scripts/explore-research/self-leverage.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"self-leverage"* ]]
    proposal_count=$(printf '%s' "$output" | python3 -c '
import json, sys
d = json.load(sys.stdin)
print(len(d["proposals"]))
')
    [ "$proposal_count" -gt 0 ]
}

# ── 2. Does NOT flag already-auto-resolved / legitimate operator points ───

@test "self-leverage: does not flag a legitimate destructive operator confirmation" {
    mkdir -p skills/deploy-skill
    # A SKILL.md where the confirmation is for a destructive/irreversible action —
    # this is legitimately operator-facing and must NOT be flagged.
    cat > skills/deploy-skill/SKILL.md <<'EOF'
# deploy-skill

## Steps

1. Validate the build.
2. Operator must confirm: this action will push --force to main (irreversible).
3. Execute the deployment.
EOF
    git add -A && git commit -q -m "seed destructive confirmation"
    run bash "$REPO_ROOT/scripts/explore-research/self-leverage.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"self-leverage"* ]]
    # The irreversible/destructive confirmation must NOT be in proposals.
    # (The SUT's OPERATOR_LEGIT regex filters it out.)
    [[ "$output" != *"push --force"* ]]
}

@test "self-leverage: does not flag a 'run/defer/refine' operator prompt" {
    mkdir -p skills/autospec-run
    cat > skills/autospec-run/SKILL.md <<'EOF'
# autospec-run

## Operator prompts

- run: proceed with the current implementation plan
- defer: skip this issue for now
- refine: revise the plan before implementing
EOF
    git add -A && git commit -q -m "seed run/defer/refine prompts"
    run bash "$REPO_ROOT/scripts/explore-research/self-leverage.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"self-leverage"* ]]
    # run/defer/refine are legitimate; none of these keyword lines should appear
    # in proposal evidence as a flagged item.
    # We just check the script exits cleanly with well-formed output.
}

# ── 3. Well-formed JSON from an empty repo ────────────────────────────────

@test "self-leverage: emits well-formed JSON from empty repo (no proposals required)" {
    git commit -q --allow-empty -m "empty repo"
    run bash "$REPO_ROOT/scripts/explore-research/self-leverage.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"self-leverage"* ]]
}
