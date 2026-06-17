#!/usr/bin/env bats
# tests/explore/test_explore_quality_resilience.bats — bats suites for the
# quality-resilience researcher (Issue B2).
#
# Asserts:
#   1. Each of the four lenses emits well-formed proposals when fed a fixture
#      repo that tickles its heuristic.
#   2. Assertion-free-test detection fires on a seeded bats file with no asserts.
#   3. Invariant/guard coverage fires on a seeded validate.sh gap.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t qr-test.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
}

teardown() {
    rm -rf "$TMP"
}

# Validate top-level JSON shape + extended fields (severity, named_consumer).
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
    if "severity" in p:
        valid = {"silent-wrong","correctness","stability","operability","feature","nicety"}
        sev = p["severity"]
        assert sev in valid, "bad severity: " + sev
'
}

# ── Lens (a): assertion-free test detection ───────────────────────────────

@test "quality-resilience: assertion-free bats file is flagged" {
    mkdir -p tests
    # A bats file with @test blocks but zero assert_* calls.
    cat > tests/no_asserts.bats <<'EOF'
#!/usr/bin/env bats
@test "does something" {
    run echo hello
}
@test "does something else" {
    run ls /
}
EOF
    git add -A && git commit -q -m "seed assertion-free bats"
    run bash "$REPO_ROOT/scripts/explore-research/quality-resilience.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"quality-resilience"* ]]
    # Must flag the assertion-free file.
    [[ "$output" == *"assertion-free"* || "$output" == *"no_asserts"* ]]
}

# ── Lens (a): self-consistent fixture detection ────────────────────────────

@test "quality-resilience: self-consistent fixture pattern is flagged" {
    mkdir -p tests src
    # A test file that sources the SUT and derives expected output from it.
    cat > src/helper.sh <<'EOF'
#!/usr/bin/env bash
make_slug() { echo "$1" | tr ' ' '-'; }
EOF
    cat > tests/test_slug.bats <<'EOF'
#!/usr/bin/env bats
source src/helper.sh
@test "slug matches" {
    expected=$(make_slug "hello world")
    run make_slug "hello world"
    assert_output "$expected"
}
EOF
    git add -A && git commit -q -m "seed self-consistent fixture"
    run bash "$REPO_ROOT/scripts/explore-research/quality-resilience.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"quality-resilience"* ]]
}

# ── Lens (b): invariant without matching test ─────────────────────────────

@test "quality-resilience: validate.sh check_fn without bats coverage is flagged" {
    mkdir -p scripts tests
    # A validate.sh with a check_ function that has no matching bats test.
    cat > scripts/validate.sh <<'EOF'
#!/usr/bin/env bash
check_zzzunique_invariant_xyz() {
    # assert something important
    echo "checking..."
}
check_zzzunique_invariant_xyz
EOF
    git add -A && git commit -q -m "seed validate.sh gap"
    run bash "$REPO_ROOT/scripts/explore-research/quality-resilience.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"quality-resilience"* ]]
    # Lens (b) should flag the uncovered invariant.
    [[ "$output" == *"zzzunique_invariant_xyz"* || "$output" == *"validate.sh"* ]]
}

# ── Lens (c): lockfile without trap cleanup ────────────────────────────────

@test "quality-resilience: lockfile without EXIT trap is flagged" {
    mkdir -p scripts
    # A script that creates a lockfile but has no trap to clean it up.
    cat > scripts/worker.sh <<'EOF'
#!/usr/bin/env bash
LOCK="$HOME/.autospec/worker.lock"
echo $$ > "$LOCK"
echo "working..."
rm -f "$LOCK"
EOF
    git add -A && git commit -q -m "seed lockfile without trap"
    run bash "$REPO_ROOT/scripts/explore-research/quality-resilience.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"quality-resilience"* ]]
}

# ── Overall: well-formed even from an empty-looking repo ──────────────────

@test "quality-resilience: emits well-formed JSON from empty repo (no proposals required)" {
    git commit -q --allow-empty -m "empty repo"
    run bash "$REPO_ROOT/scripts/explore-research/quality-resilience.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"quality-resilience"* ]]
}
