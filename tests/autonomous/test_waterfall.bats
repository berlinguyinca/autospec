#!/usr/bin/env bats
# tests/autonomous/test_waterfall.bats — unit tests for autonomous-waterfall.sh
#
# Tier 0: control label always preempts.
# Tier 1: backlog present → run-backlog; empty + dry-cycles < threshold → stay Tier 1.
# Tier 2: Tier 1 dry >= threshold → run-explore-once (local sources).
# Tier 3: Tier 2 dry >= threshold → run-explore-once-internet.
# Refill: non-empty backlog always floats back to Tier 1 regardless of dry-cycles.
# gh is stubbed via PATH injection; no real network calls.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autonomous-waterfall.sh"

    # Isolated temp dir — no git, no gh needed for inject path.
    TMP="$(mktemp -d -t waterfall.XXXXXX)"
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin"

    # Default stub: gh returns 0 issues.
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
# Stub gh: return 0 for list --json number --jq 'length'
printf '0\n'
exit 0
EOF
    chmod +x "$TMP/bin/gh"
}

teardown() {
    rm -rf "$TMP"
}

# ─── Tier 0 tests ──────────────────────────────────────────────────────────────

@test "Tier 0: control label preempts when backlog count would be non-zero" {
    run bash "$SCRIPT" \
        --control-decision "autospec:pause" \
        --backlog-count 5
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":0'* ]]
    [[ "$output" == *'"action":"control"'* ]]
    [[ "$output" == *'autospec:pause'* ]]
}

@test "Tier 0: control label preempts even with empty backlog" {
    run bash "$SCRIPT" \
        --control-decision "autospec:stop" \
        --backlog-count 0
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":0'* ]]
    [[ "$output" == *'"action":"control"'* ]]
}

@test "Tier 0: any non-empty control-decision value triggers Tier 0" {
    run bash "$SCRIPT" \
        --control-decision "autospec:steer" \
        --backlog-count 0 \
        --dry-cycles 99
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":0'* ]]
}

# ─── Tier 1 tests ──────────────────────────────────────────────────────────────

@test "Tier 1: backlog present selects run-backlog" {
    run bash "$SCRIPT" \
        --backlog-count 3
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":1'* ]]
    [[ "$output" == *'"action":"run-backlog"'* ]]
}

@test "Tier 1: backlog present selects Tier 1 even with high dry-cycles" {
    run bash "$SCRIPT" \
        --backlog-count 10 \
        --dry-cycles 99
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":1'* ]]
}

@test "Tier 1: backlog empty + 1 dry cycle stays Tier 1 (default threshold=2)" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --dry-cycles 1
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":1'* ]]
    [[ "$output" == *'"action":"run-backlog"'* ]]
}

@test "Tier 1: backlog empty + 0 dry cycles stays Tier 1" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --dry-cycles 0
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":1'* ]]
}

# ─── Tier 2 escalation ─────────────────────────────────────────────────────────

@test "Tier 2: Tier-1 dry x2 (default threshold) selects run-explore-once" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --dry-cycles 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":2'* ]]
    [[ "$output" == *'"action":"run-explore-once"'* ]]
}

@test "Tier 2: Tier-1 dry above threshold also selects Tier 2 when Tier-2 counter is 0" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --dry-cycles 5 \
        --tier2-dry-cycles 0
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":2'* ]]
    [[ "$output" == *'"action":"run-explore-once"'* ]]
}

@test "Tier 2: Tier-1 dry >= threshold, Tier-2 dry < threshold stays at Tier 2" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --dry-cycles 2 \
        --tier2-dry-cycles 1
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":2'* ]]
    [[ "$output" == *'"action":"run-explore-once"'* ]]
}

@test "Tier 2: custom threshold via AUTOSPEC_AUTO_DRY_CYCLES, dry-cycles below stays Tier 1" {
    AUTOSPEC_AUTO_DRY_CYCLES=3 run bash "$SCRIPT" \
        --backlog-count 0 \
        --dry-cycles 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":1'* ]]
}

@test "Tier 2: custom threshold=3 escalates at dry-cycles=3" {
    AUTOSPEC_AUTO_DRY_CYCLES=3 run bash "$SCRIPT" \
        --backlog-count 0 \
        --dry-cycles 3 \
        --tier2-dry-cycles 0
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":2'* ]]
    [[ "$output" == *'"action":"run-explore-once"'* ]]
}

# ─── Tier 3 escalation ─────────────────────────────────────────────────────────

@test "Tier 3: Tier-2 dry x2 selects run-explore-once-internet" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --dry-cycles 2 \
        --tier2-dry-cycles 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":3'* ]]
    [[ "$output" == *'"action":"run-explore-once-internet"'* ]]
}

@test "Tier 3: Tier-2 dry above threshold also selects Tier 3" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --dry-cycles 10 \
        --tier2-dry-cycles 5
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":3'* ]]
    [[ "$output" == *'"action":"run-explore-once-internet"'* ]]
}

@test "Tier 3: custom threshold=3 escalates at tier2-dry-cycles=3" {
    AUTOSPEC_AUTO_DRY_CYCLES=3 run bash "$SCRIPT" \
        --backlog-count 0 \
        --dry-cycles 3 \
        --tier2-dry-cycles 3
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":3'* ]]
    [[ "$output" == *'"action":"run-explore-once-internet"'* ]]
}

# ─── Refill floats back to Tier 1 ─────────────────────────────────────────────
# When a higher tier files candidates they become backlog; next cycle the
# waterfall sees backlog_count > 0 and selects Tier 1 regardless of dry-cycles.

@test "Refill: non-empty backlog overrides high dry-cycles and tier2-dry-cycles" {
    run bash "$SCRIPT" \
        --backlog-count 1 \
        --dry-cycles 99 \
        --tier2-dry-cycles 99
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":1'* ]]
    [[ "$output" == *'"action":"run-backlog"'* ]]
}

@test "Refill: backlog-count 3 after Tier 3 escalation returns to Tier 1" {
    run bash "$SCRIPT" \
        --backlog-count 3 \
        --dry-cycles 5 \
        --tier2-dry-cycles 5
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":1'* ]]
}

# ─── Output format ─────────────────────────────────────────────────────────────

@test "output is valid JSON with tier, action, reason keys" {
    run bash "$SCRIPT" --backlog-count 1
    [ "$status" -eq 0 ]
    # All three keys must be present.
    [[ "$output" == *'"tier":'* ]]
    [[ "$output" == *'"action":'* ]]
    [[ "$output" == *'"reason":'* ]]
}

@test "Tier 2 output contains tier, action, reason keys" {
    run bash "$SCRIPT" --backlog-count 0 --dry-cycles 2 --tier2-dry-cycles 0
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":'* ]]
    [[ "$output" == *'"action":'* ]]
    [[ "$output" == *'"reason":'* ]]
}

@test "Tier 3 output contains tier, action, reason keys" {
    run bash "$SCRIPT" --backlog-count 0 --dry-cycles 2 --tier2-dry-cycles 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":'* ]]
    [[ "$output" == *'"action":'* ]]
    [[ "$output" == *'"reason":'* ]]
}

@test "unknown flag exits 2" {
    run bash "$SCRIPT" --unknown-flag
    [ "$status" -eq 2 ]
}

# ─── gh subprocess boundary ────────────────────────────────────────────────────

@test "gh stub returning 5 is treated as backlog present (Tier 1)" {
    # Override stub to return 5 issues.
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '5\n'
exit 0
EOF
    chmod +x "$TMP/bin/gh"

    # Use a fake git dir so repo detection gets a slug.
    git_dir="$(mktemp -d -t wf-git.XXXXXX)"
    git -C "$git_dir" init -q
    git -C "$git_dir" remote add origin "https://github.com/owner/repo.git"

    run bash "$SCRIPT" --repo "owner/repo"
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":1'* ]]
    rm -rf "$git_dir"
}

@test "gh failure degrades gracefully to empty backlog (stays Tier 1 at dry-cycles=0)" {
    # Override stub to fail.
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
    chmod +x "$TMP/bin/gh"

    run bash "$SCRIPT" --repo "owner/repo" --dry-cycles 0
    [ "$status" -eq 0 ]
    # gh failure → backlog_count=0 → dry-cycles=0 < 2 → still Tier 1
    [[ "$output" == *'"tier":1'* ]]
}
