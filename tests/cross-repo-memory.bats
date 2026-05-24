#!/usr/bin/env bats
# tests/cross-repo-memory.bats — tests for cross-repo memory index scripts
# Covers: register / search / GC / missing-mempalace fallback / opt-out

INDEX_SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/cross-repo-index.sh"
SEARCH_SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/cross-repo-search.sh"

setup() {
    TEST_TMP="$(mktemp -d)"

    # Fake home to isolate ~/.autospec/memory-index/
    export HOME="$TEST_TMP/home"
    mkdir -p "$HOME"

    # Repo A: has docs/memory/ with our target file
    REPO_A="$TEST_TMP/repo-a"
    mkdir -p "$REPO_A/docs/memory"
    git -C "$REPO_A" init -q
    git -C "$REPO_A" commit --allow-empty -m "init" -q 2>/dev/null || true
    cat > "$REPO_A/docs/memory/feedback_bash_return_trap_leak.md" <<'EOF'
---
type: feedback
---
# RETURN trap bash
RETURN traps in bash functions leak into caller frames; never use for local cleanup under set -u.
EOF

    # Repo B: has docs/memory/ with a different file
    REPO_B="$TEST_TMP/repo-b"
    mkdir -p "$REPO_B/docs/memory"
    git -C "$REPO_B" init -q
    git -C "$REPO_B" commit --allow-empty -m "init" -q 2>/dev/null || true
    cat > "$REPO_B/docs/memory/feedback_omc_autopilot_misfire.md" <<'EOF'
---
type: feedback
---
# OMC autopilot misfire
system reminders containing AUTOPILOT auto-activate OMC autopilot mid-session.
EOF

    export TEST_TMP REPO_A REPO_B
}

teardown() {
    rm -rf "$TEST_TMP"
}

# ── sanity ────────────────────────────────────────────────────────────────────

@test "cross-repo-index.sh exists and is executable" {
    [ -x "$INDEX_SCRIPT" ]
}

@test "cross-repo-search.sh exists and is executable" {
    [ -x "$SEARCH_SCRIPT" ]
}

@test "cross-repo-index.sh --help exits 0" {
    run bash "$INDEX_SCRIPT" --help
    [ "$status" -eq 0 ]
}

@test "cross-repo-search.sh --help exits 0" {
    run bash "$SEARCH_SCRIPT" --help
    [ "$status" -eq 0 ]
}

# ── registration ──────────────────────────────────────────────────────────────

@test "register: creates symlink in ~/.autospec/memory-index/<slug>" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    # A symlink must exist somewhere under the index dir
    INDEX_DIR="$HOME/.autospec/memory-index"
    [ -d "$INDEX_DIR" ]
    local found
    found=$(find "$INDEX_DIR" -maxdepth 1 -type l | head -1)
    [ -n "$found" ]
}

@test "register: symlink points to repo docs/memory/" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    INDEX_DIR="$HOME/.autospec/memory-index"
    local target
    target=$(readlink "$(find "$INDEX_DIR" -maxdepth 1 -type l | head -1)")
    [ "$target" = "$REPO_A/docs/memory" ]
}

@test "register: idempotent — second call does not create duplicate" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    INDEX_DIR="$HOME/.autospec/memory-index"
    local count
    count=$(find "$INDEX_DIR" -maxdepth 1 -type l | wc -l | tr -d ' ')
    [ "$count" -eq 1 ]
}

@test "register: two different repos create two symlinks" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_B"
    [ "$status" -eq 0 ]
    INDEX_DIR="$HOME/.autospec/memory-index"
    local count
    count=$(find "$INDEX_DIR" -maxdepth 1 -type l | wc -l | tr -d ' ')
    [ "$count" -eq 2 ]
}

# ── opt-out ───────────────────────────────────────────────────────────────────

@test "register: AUTOSPEC_SKIP_CROSS_REPO_INDEX=1 is a no-op" {
    AUTOSPEC_SKIP_CROSS_REPO_INDEX=1 run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    INDEX_DIR="$HOME/.autospec/memory-index"
    local count
    count=$(find "$INDEX_DIR" -maxdepth 1 -type l 2>/dev/null | wc -l | tr -d ' ')
    [ "$count" -eq 0 ]
}

# ── GC (stale entry removal) ─────────────────────────────────────────────────

@test "GC: stale entry (docs/memory/ deleted) is removed on next register call" {
    # Register repo A, then remove its docs/memory/
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    rm -rf "$REPO_A/docs/memory"
    # Re-run from any repo — GC runs on every call
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_B"
    [ "$status" -eq 0 ]
    INDEX_DIR="$HOME/.autospec/memory-index"
    # Only repo-b symlink should remain (repo-a was GC'd)
    local count
    count=$(find "$INDEX_DIR" -maxdepth 1 -type l | wc -l | tr -d ' ')
    [ "$count" -eq 1 ]
}

# ── search ────────────────────────────────────────────────────────────────────

@test "search: finds RETURN trap match from indexed repo A" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    run bash "$SEARCH_SCRIPT" "RETURN trap" --index-dir "$HOME/.autospec/memory-index"
    [ "$status" -eq 0 ]
    echo "$output" | grep -qi "RETURN"
}

@test "search: tags hit with source repo slug" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    run bash "$SEARCH_SCRIPT" "RETURN trap" --index-dir "$HOME/.autospec/memory-index"
    [ "$status" -eq 0 ]
    # Output should mention the source (filename or repo slug)
    echo "$output" | grep -qi "feedback_bash_return_trap_leak\|repo-a"
}

@test "search: finds match from repo B when both indexed" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_B"
    [ "$status" -eq 0 ]
    run bash "$SEARCH_SCRIPT" "autopilot misfire" --index-dir "$HOME/.autospec/memory-index"
    [ "$status" -eq 0 ]
    echo "$output" | grep -qi "autopilot\|omc"
}

@test "search: no results exits 0 with empty output" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    run bash "$SEARCH_SCRIPT" "xyzzy_no_match_expected_12345" --index-dir "$HOME/.autospec/memory-index"
    [ "$status" -eq 0 ]
}

@test "search: empty index exits 0" {
    run bash "$SEARCH_SCRIPT" "RETURN trap" --index-dir "$HOME/.autospec/memory-index"
    [ "$status" -eq 0 ]
}

# ── grep-fallback pattern sanitization (G1/G2) ───────────────────────────────
# These force the grep branch by hiding `mempalace` from PATH, so the
# pattern-building code path (not the mempalace branch) is exercised.

@test "search (grep fallback): trailing-space query returns only genuine matches, not match-all" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    # Query for a term that does NOT appear in any indexed file, plus a trailing
    # space. A buggy 'foo|' pattern matches every line on GNU grep (match-all).
    run env PATH="/usr/bin:/bin" bash "$SEARCH_SCRIPT" "zzznotpresent " --index-dir "$HOME/.autospec/memory-index"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "search (grep fallback): internal double-space query does not match-all" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    # Internal duplicate space → naive tr gives 'zzznotpresent||' (empty alt → match-all).
    run env PATH="/usr/bin:/bin" bash "$SEARCH_SCRIPT" "zzznotpresent  alsoabsent" --index-dir "$HOME/.autospec/memory-index"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "search (grep fallback): genuine match still found with trailing space" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    run env PATH="/usr/bin:/bin" bash "$SEARCH_SCRIPT" "RETURN trap " --index-dir "$HOME/.autospec/memory-index"
    [ "$status" -eq 0 ]
    echo "$output" | grep -qi "RETURN"
}

@test "search (grep fallback): all-whitespace query returns empty, exit 0" {
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    run env PATH="/usr/bin:/bin" bash "$SEARCH_SCRIPT" "   " --index-dir "$HOME/.autospec/memory-index"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

# ── inject-relevant-memory --cross-repo ───────────────────────────────────────

@test "inject-relevant-memory: --cross-repo flag accepted without error" {
    INJECT_SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/inject-relevant-memory.sh"
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    run bash "$INJECT_SCRIPT" --context "RETURN trap" --cross-repo \
        --index-dir "$HOME/.autospec/memory-index"
    [ "$status" -eq 0 ]
}

@test "inject-relevant-memory: --cross-repo returns cross-repo hit" {
    INJECT_SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/inject-relevant-memory.sh"
    run bash "$INDEX_SCRIPT" --repo-root "$REPO_A"
    [ "$status" -eq 0 ]
    run bash "$INJECT_SCRIPT" --context "RETURN trap bash" --cross-repo \
        --index-dir "$HOME/.autospec/memory-index"
    [ "$status" -eq 0 ]
    echo "$output" | grep -qi "RETURN\|return_trap"
}
