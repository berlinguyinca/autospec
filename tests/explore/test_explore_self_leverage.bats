#!/usr/bin/env bats
# tests/explore/test_explore_self_leverage.bats — bats suite for the
# self-leverage researcher (Issue B2; precision refinement 2026-06-26).
#
# New contract (precision refinement): self-leverage scans ONLY executable
# scripts for real `read -p`/`-rp` PROMPT statements — never markdown prose,
# never bare `read -r`, never comment/help-text token mentions. Each proposal
# carries a kind=present gap_check the aggregator re-confirms.
#
# Asserts:
#   1. Does NOT flag human-in-loop PROSE in a SKILL.md (the old false-positive).
#   2. Flags a real `read -p` prompt in a script, with a present gap_check.
#   3. Does NOT flag a destructive `read -p` (OPERATOR_LEGIT filters it).
#   4. Does NOT flag a bare `read -r` / comment mention of --interactive.
#   5. Well-formed JSON from an empty repo.

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

proposal_count() {
    printf '%s' "$1" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["proposals"]))'
}

# ── 1. Does NOT flag human-in-loop PROSE (the eliminated false positive) ──

@test "self-leverage: does NOT flag human-in-loop prose in a SKILL.md" {
    mkdir -p skills/my-skill
    # Prose describing a human-in-loop step. Under the old behavior this was
    # flagged (a false positive — prose is not code); now prose is never scanned.
    cat > skills/my-skill/SKILL.md <<'EOF'
# my-skill

## Steps

1. Gather inputs.
2. The operator must manually confirm the config file path before proceeding.
3. Continue with the rest of the process.
EOF
    git add -A && git commit -q -m "seed prose intervention"
    run bash "$REPO_ROOT/scripts/explore-research/self-leverage.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [ "$(proposal_count "$output")" -eq 0 ]
}

# ── 2. Flags a real read -p prompt in a script, with a present gap_check ──

@test "self-leverage: flags a real read -p prompt in a script" {
    mkdir -p scripts
    cat > scripts/helper.sh <<'EOF'
#!/usr/bin/env bash
read -p "Enter the config name: " config_name
echo "Using config: $config_name"
EOF
    git add -A && git commit -q -m "seed read -p prompt"
    run bash "$REPO_ROOT/scripts/explore-research/self-leverage.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [ "$(proposal_count "$output")" -gt 0 ]
    # Each proposal must carry a kind=present gap_check pointing at the script.
    printf '%s' "$output" | python3 -c '
import json, sys
d = json.load(sys.stdin)
p = d["proposals"][0]
gc = p.get("gap_check", {})
assert gc.get("kind") == "present", gc
assert gc.get("haystack") == "scripts/helper.sh", gc
assert "scripts/helper.sh" in p["title"], p["title"]
'
}

# ── 3. Does NOT flag a destructive read -p (OPERATOR_LEGIT filters it) ────

@test "self-leverage: does not flag a destructive read -p confirmation" {
    mkdir -p scripts
    cat > scripts/deploy.sh <<'EOF'
#!/usr/bin/env bash
read -p "Confirm: this will push --force to main (irreversible)? " ok
EOF
    git add -A && git commit -q -m "seed destructive read -p"
    run bash "$REPO_ROOT/scripts/explore-research/self-leverage.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" != *"push --force"* ]]
    [ "$(proposal_count "$output")" -eq 0 ]
}

# ── 4. Does NOT flag bare read -r or a comment mention of --interactive ───

@test "self-leverage: does not flag bare read -r or a comment token" {
    mkdir -p scripts
    cat > scripts/loop.sh <<'EOF'
#!/usr/bin/env bash
# supports --interactive mode and AskUserQuestion in docs
while IFS= read -r line; do
    echo "$line"
done < input.txt
EOF
    git add -A && git commit -q -m "seed bare read -r"
    run bash "$REPO_ROOT/scripts/explore-research/self-leverage.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [ "$(proposal_count "$output")" -eq 0 ]
}

# ── 3. Well-formed JSON from an empty repo ────────────────────────────────

@test "self-leverage: emits well-formed JSON from empty repo (no proposals required)" {
    git commit -q --allow-empty -m "empty repo"
    run bash "$REPO_ROOT/scripts/explore-research/self-leverage.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"self-leverage"* ]]
}
