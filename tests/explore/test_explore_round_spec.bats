#!/usr/bin/env bats
# tests/explore/test_explore_round_spec.bats — deterministic round-spec renderer
# TDD: tests for scripts/gen-explore-round-spec.sh

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t round-spec.XXXXXX)"

    # Write a 2-proposal fixture
    cat > "$TMP/proposals.json" <<'EOF'
{
  "round": "2026-06-16",
  "proposals": [
    {
      "title": "feat: add retry logic to explore loop",
      "evidence": "scripts/autospec-explore.sh:42",
      "estimated_complexity": "small",
      "confidence": 0.9,
      "source": "spec-vs-code",
      "severity": "correctness",
      "named_consumer": "autospec-run"
    },
    {
      "title": "fix: validate proposal schema on emit",
      "evidence": "scripts/explore-research-cycle.sh:88",
      "estimated_complexity": "medium",
      "confidence": 0.75,
      "source": "codebase-signals",
      "severity": "stability",
      "named_consumer": "autospec-explore"
    }
  ]
}
EOF

    # Write an empty-proposals fixture
    cat > "$TMP/empty.json" <<'EOF'
{
  "round": "2026-06-16",
  "proposals": []
}
EOF
}

teardown() {
    rm -rf "$TMP"
}

# ── basic rendering ────────────────────────────────────────────────────────────

@test "exits 0 on a valid 2-proposal JSON fixture" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
}

@test "output contains exactly one ## section per proposal" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    count=$(printf '%s\n' "$output" | grep -c '^## ')
    [ "$count" -eq 2 ]
}

@test "each proposal section title appears as a ## heading" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^## feat: add retry logic to explore loop'
    printf '%s\n' "$output" | grep -q '^## fix: validate proposal schema on emit'
}

@test "each proposal section contains at least 1 - [ ] acceptance checkbox" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    count=$(printf '%s\n' "$output" | grep -c '^\- \[ \]')
    [ "$count" -ge 2 ]
}

@test "output header names the sandbox branch" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'demo'
}

@test "output header names the round number" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 3 --branch mybranch
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'round 3'
}

@test "output contains evidence field value" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'scripts/autospec-explore.sh:42'
}

@test "output contains source field value" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'spec-vs-code'
}

@test "output contains severity field value" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'correctness'
}

@test "output contains named_consumer field value" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'autospec-run'
}

@test "output contains estimated_complexity field value" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'small'
}

# ── determinism ────────────────────────────────────────────────────────────────

@test "re-running on same fixture yields byte-identical output" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    first="$output"

    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    [ "$output" = "$first" ]
}

# ── --out flag ────────────────────────────────────────────────────────────────

@test "--out writes to a file and stdout is empty" {
    out_file="$TMP/spec.md"
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/proposals.json" --round 1 --branch demo --out "$out_file"
    [ "$status" -eq 0 ]
    [ -f "$out_file" ]
    grep -q '^\- \[ \]' "$out_file"
}

# ── edge cases ────────────────────────────────────────────────────────────────

@test "empty proposals produces minimal valid spec with # header" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        "$TMP/empty.json" --round 1 --branch demo
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^# '
}

@test "missing proposals JSON file exits non-zero" {
    run bash "$REPO_ROOT/scripts/gen-explore-round-spec.sh" \
        /nonexistent/path.json --round 1 --branch demo
    [ "$status" -ne 0 ]
}
