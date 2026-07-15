#!/usr/bin/env bats
# tests/autonomous/test_waterfall.bats — unit tests for autonomous-waterfall.sh
#
# Tier 0: control label always preempts.
# Tier 1: backlog present → run-backlog; empty + dry-cycles < threshold → stay Tier 1.
# Tier 1.5: Tier 1 dry >= threshold + open issues → promote open issues.
# Tier 2: promotion dry + Tier 1 dry >= threshold → run-explore-once (local sources).
# Tier 3: Tier 2 dry >= threshold → architecture/test-coverage improvement.
# Tier 4: Tier 3 dry >= threshold → run-explore-once-internet; park only after Tier 4 dry.
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

# ─── Tier 1.5 promotion / Tier 2 local discovery ─────────────────────────────

@test "Tier 1.5: Tier-1 dry x2 with open issues promotes instead of parking" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --open-issue-count 3 \
        --dry-cycles 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":1.5'* ]]
    [[ "$output" == *'"action":"promote-open-issues"'* ]]
}

@test "Tier 2: default waterfall enters local discovery after promotion is dry" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --open-issue-count 0 \
        --dry-cycles 2 \
        --tier15-dry-cycles 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":2'* ]]
    [[ "$output" == *'"action":"run-explore-once"'* ]]
}

@test "Tier 2: dry above threshold selects Tier 2 when Tier-2 counter is 0" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --open-issue-count 0 \
        --dry-cycles 5 \
        --tier15-dry-cycles 5 \
        --tier2-dry-cycles 0
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":2'* ]]
    [[ "$output" == *'"action":"run-explore-once"'* ]]
}

@test "Tier 2: Tier-2 dry below threshold remains at local discovery" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --open-issue-count 0 \
        --dry-cycles 2 \
        --tier15-dry-cycles 2 \
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

@test "Tier 2: custom threshold=3 escalates at dry-cycles=3 after promotion dry" {
    AUTOSPEC_AUTO_DRY_CYCLES=3 run bash "$SCRIPT" \
        --backlog-count 0 \
        --open-issue-count 0 \
        --dry-cycles 3 \
        --tier15-dry-cycles 3 \
        --tier2-dry-cycles 0
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":2'* ]]
    [[ "$output" == *'"action":"run-explore-once"'* ]]
}

# ─── Tier 3/4 escalation ──────────────────────────────────────────────────────

@test "Tier 3: Tier-2 dry x2 selects architecture improvement" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --open-issue-count 0 \
        --dry-cycles 2 \
        --tier15-dry-cycles 2 \
        --tier2-dry-cycles 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":3'* ]]
    [[ "$output" == *'"action":"run-architecture-improvement"'* ]]
}

@test "Tier 3: Tier-2 dry above threshold selects architecture improvement" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --open-issue-count 0 \
        --dry-cycles 10 \
        --tier15-dry-cycles 10 \
        --tier2-dry-cycles 5
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":3'* ]]
    [[ "$output" == *'"action":"run-architecture-improvement"'* ]]
}

@test "Tier 4: Tier-3 dry x2 selects internet discovery" {
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --open-issue-count 0 \
        --dry-cycles 2 \
        --tier15-dry-cycles 2 \
        --tier2-dry-cycles 2 \
        --tier3-dry-cycles 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":4'* ]]
    [[ "$output" == *'"action":"run-explore-once-internet"'* ]]
}

@test "Idle-rescan: all tiers dry idles-and-rescans instead of convergence-parking" {
    # Never-idle contract (R1/R5, F1 of the 2026-07-06 platform design): a fully
    # dry cascade must enter idle-rescan, NOT terminate. Only resource/control
    # park may exit the loop — convergence-stop is forbidden.
    run bash "$SCRIPT" \
        --backlog-count 0 \
        --open-issue-count 0 \
        --dry-cycles 2 \
        --tier15-dry-cycles 2 \
        --tier2-dry-cycles 2 \
        --tier3-dry-cycles 2 \
        --tier4-dry-cycles 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":4'* ]]
    [[ "$output" == *'"action":"idle-rescan"'* ]]
    [[ "$output" == *'all tiers dry'* ]]
    # Must NOT convergence-park.
    [[ "$output" != *'"action":"park"'* ]]
}

@test "Emergency kill-switch: AUTOSPEC_DISABLE_DISCOVERY_TIERS=1 still parks (distinct from idle-rescan)" {
    # The documented fail-closed emergency park must remain park, NOT idle-rescan.
    AUTOSPEC_DISABLE_DISCOVERY_TIERS=1 run bash "$SCRIPT" \
        --backlog-count 0 \
        --dry-cycles 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"action":"park"'* ]]
    [[ "$output" == *'discovery tiers disabled'* ]]
    [[ "$output" != *'"action":"idle-rescan"'* ]]
}

# ─── Refill floats back to Tier 1 ─────────────────────────────────────────────
# When a higher tier files candidates they become backlog; next cycle the
# waterfall sees backlog_count > 0 and selects Tier 1 regardless of dry-cycles.

@test "Refill: non-empty backlog overrides high dry-cycles and tier2-dry-cycles" {
    run bash "$SCRIPT" \
        --backlog-count 1 \
        --dry-cycles 99 \
        --tier15-dry-cycles 99 \
        --tier2-dry-cycles 99 \
        --tier3-dry-cycles 99 \
        --tier4-dry-cycles 99
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":1'* ]]
    [[ "$output" == *'"action":"run-backlog"'* ]]
}

@test "Refill: backlog-count 3 after higher-tier escalation returns to Tier 1" {
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

@test "idle-rescan output contains tier, action, reason keys" {
    run bash "$SCRIPT" --backlog-count 0 --open-issue-count 0 --dry-cycles 2 --tier15-dry-cycles 2 --tier2-dry-cycles 2 --tier3-dry-cycles 2 --tier4-dry-cycles 2
    [ "$status" -eq 0 ]
    [[ "$output" == *'"tier":'* ]]
    [[ "$output" == *'"action":'* ]]
    [[ "$output" == *'"reason":'* ]]
}

@test "Tier 3 output contains tier, action, reason keys" {
    run bash "$SCRIPT" --backlog-count 0 --open-issue-count 0 --dry-cycles 2 --tier15-dry-cycles 2 --tier2-dry-cycles 2
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

# ─── Defense-in-depth: standalone readiness-aware backlog_count (#1632) ───────
# When BACKLOG_COUNT_INJECT is not supplied (waterfall invoked standalone),
# the naive `gh` open-auto-implement count must NOT gate Tier-1 by itself —
# the waterfall must consult autospec queue ready (dependency-aware) and use
# ready+batch as backlog_count, falling back to the naive gh count only when
# the Rust queue is unavailable.

@test "defense-in-depth: no --backlog-count, naive gh count > 0 but Rust queue reports all-blocked -> advances past Tier 1" {
    # Naive gh count would be 1 (misleading — this must NOT pin Tier 1).
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '1\n'
exit 0
EOF
    chmod +x "$TMP/bin/gh"

    # Dependency-aware Rust queue: nothing ready, one blocked.
    cat > "$TMP/bin/autospec" <<'EOF'
#!/usr/bin/env bash
printf '{"ready":[],"blocked":[{"number":42}],"claimed":[],"conflicts":[],"worker_cap":{"reached":false},"batch":[]}\n'
EOF
    chmod +x "$TMP/bin/autospec"
    export AUTOSPEC_QUEUE_BIN="$TMP/bin/autospec"

    run bash "$SCRIPT" --repo "owner/repo" --dry-cycles 2 --open-issue-count 0
    [ "$status" -eq 0 ]
    # backlog_count resolved to 0 (readiness-aware) -> dry-cycles=2 >= threshold
    # -> advances past Tier 1 (Tier 1.5 promote, or deeper/park).
    [[ "$output" != *'"tier":1,'* ]]
}

@test "defense-in-depth: Rust queue unavailable falls back to naive gh count" {
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '1\n'
exit 0
EOF
    chmod +x "$TMP/bin/gh"
    unset AUTOSPEC_QUEUE_BIN
    rm -f "$TMP/bin/autospec"

    run bash "$SCRIPT" --repo "owner/repo" --dry-cycles 2
    [ "$status" -eq 0 ]
    # No readiness helper available -> naive gh count (1) is used -> Tier 1.
    [[ "$output" == *'"tier":1'* ]]
    [[ "$output" == *'"action":"run-backlog"'* ]]
}

# ─── Worker-cap-reached is treated as dry, not as ready backlog (#1632) ───────

@test "worker-cap-reached: Rust queue reports ready issues but worker cap reached -> treated as dry" {
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '3\n'
exit 0
EOF
    chmod +x "$TMP/bin/gh"

    cat > "$TMP/bin/autospec" <<'EOF'
#!/usr/bin/env bash
printf '{"ready":[{"number":1},{"number":2}],"blocked":[],"claimed":[],"conflicts":[],"worker_cap":{"reached":true},"batch":[]}\n'
EOF
    chmod +x "$TMP/bin/autospec"
    export AUTOSPEC_QUEUE_BIN="$TMP/bin/autospec"

    run bash "$SCRIPT" --repo "owner/repo" --dry-cycles 2 --open-issue-count 0
    [ "$status" -eq 0 ]
    # Worker cap reached -> readiness-aware count is forced to 0 -> dry -> advances.
    [[ "$output" != *'"tier":1,'* ]]
}
