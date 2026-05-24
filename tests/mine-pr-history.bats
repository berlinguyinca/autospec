#!/usr/bin/env bats
# tests/mine-pr-history.bats — tests for skills/autospec-shared/scripts/mine-pr-history.sh
# Covers: creates lesson files, idempotency, frontmatter linking, dedup, filter, missing-gh fallback

SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/mine-pr-history.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    MEMORY_DIR="$TEST_TMP/docs/memory"
    mkdir -p "$MEMORY_DIR"

    # Stub gh: returns two fake PRs with lesson: in body (bodies > 200 chars) + one short PR
    mkdir -p "$TEST_TMP/bin"
    # Write the stub JSON response to a file so we can include long strings cleanly
    cat > "$TEST_TMP/gh-response.json" <<'JSONEOF'
[
  {"number":42,"title":"fix: something useful","url":"https://github.com/org/repo/pull/42","body":"lesson: always check return codes. This is a detailed explanation that goes well beyond two hundred characters to ensure it passes the minimum body length filter applied by the mining script when scanning merged pull requests for useful lessons."},
  {"number":99,"title":"feat: another thing","url":"https://github.com/org/repo/pull/99","body":"lesson: prefer idempotent operations. When designing scripts and automation, always prefer operations that can be safely repeated without side effects. This body is long enough to pass the 200 char filter threshold."},
  {"number":7,"title":"short","url":"https://github.com/org/repo/pull/7","body":"x"}
]
JSONEOF
    cat > "$TEST_TMP/bin/gh" <<GHSTUB
#!/usr/bin/env bash
if [[ "\$*" == *"pr list"* ]]; then
    cat "$TEST_TMP/gh-response.json"
    exit 0
fi
exit 0
GHSTUB
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
    export TEST_TMP MEMORY_DIR
}

teardown() {
    rm -rf "$TEST_TMP"
}

# ── basic sanity ──────────────────────────────────────────────────────────────

@test "mine-pr-history.sh exists and is executable" {
    [ -x "$SCRIPT" ]
}

@test "mine-pr-history.sh --help exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
}

# ── produces lesson files ─────────────────────────────────────────────────────

@test "creates lesson files for PRs matching body:lesson:" {
    run env AUTOSPEC_MINE_PR_HISTORY=1 bash "$SCRIPT" --repo org/repo --output-dir "$MEMORY_DIR"
    [ "$status" -eq 0 ]
    # PRs 42 and 99 have "lesson:" in body; PR 7 body is too short
    [ "$(ls "$MEMORY_DIR"/lesson_*.md 2>/dev/null | wc -l)" -ge 2 ]
}

@test "each lesson file has frontmatter with source PR URL" {
    env AUTOSPEC_MINE_PR_HISTORY=1 bash "$SCRIPT" --repo org/repo --output-dir "$MEMORY_DIR"
    for f in "$MEMORY_DIR"/lesson_*.md; do
        [ -f "$f" ] || continue
        grep -q "source_pr:" "$f" || grep -q "https://github.com" "$f"
    done
}

@test "each lesson file has type: feedback frontmatter" {
    env AUTOSPEC_MINE_PR_HISTORY=1 bash "$SCRIPT" --repo org/repo --output-dir "$MEMORY_DIR"
    for f in "$MEMORY_DIR"/lesson_*.md; do
        [ -f "$f" ] || continue
        grep -q "type:" "$f"
    done
}

# ── idempotency ───────────────────────────────────────────────────────────────

@test "second run skips already-mined PRs" {
    env AUTOSPEC_MINE_PR_HISTORY=1 bash "$SCRIPT" --repo org/repo --output-dir "$MEMORY_DIR"
    local count_before
    count_before=$(ls "$MEMORY_DIR"/lesson_*.md 2>/dev/null | wc -l)
    env AUTOSPEC_MINE_PR_HISTORY=1 bash "$SCRIPT" --repo org/repo --output-dir "$MEMORY_DIR"
    local count_after
    count_after=$(ls "$MEMORY_DIR"/lesson_*.md 2>/dev/null | wc -l)
    [ "$count_before" -eq "$count_after" ]
}

# ── filter: body too short ────────────────────────────────────────────────────

@test "PR with body shorter than 200 chars is skipped" {
    env AUTOSPEC_MINE_PR_HISTORY=1 bash "$SCRIPT" --repo org/repo --output-dir "$MEMORY_DIR"
    # PR 7 has body "x" (1 char) — should NOT produce a lesson file
    ls "$MEMORY_DIR"/lesson_*7*.md 2>/dev/null && return 1 || return 0
}

# ── missing gh: graceful fallback ────────────────────────────────────────────

@test "missing gh CLI: exits 0 silently" {
    rm -f "$TEST_TMP/bin/gh"
    run bash "$SCRIPT" --repo org/repo --output-dir "$MEMORY_DIR"
    [ "$status" -eq 0 ]
}

@test "missing gh CLI: no lesson files created" {
    rm -f "$TEST_TMP/bin/gh"
    bash "$SCRIPT" --repo org/repo --output-dir "$MEMORY_DIR"
    [ "$(ls "$MEMORY_DIR"/lesson_*.md 2>/dev/null | wc -l)" -eq 0 ]
}

# ── missing --repo: exits 0 ───────────────────────────────────────────────────

@test "missing --repo: exits 0 (no-op)" {
    run bash "$SCRIPT" --output-dir "$MEMORY_DIR"
    [ "$status" -eq 0 ]
}

# ── AUTOSPEC_MINE_PR_HISTORY flag ─────────────────────────────────────────────

@test "without AUTOSPEC_MINE_PR_HISTORY=1 flag: exits 0 without mining" {
    run env -u AUTOSPEC_MINE_PR_HISTORY bash "$SCRIPT" --repo org/repo --output-dir "$MEMORY_DIR"
    [ "$status" -eq 0 ]
    [ "$(ls "$MEMORY_DIR"/lesson_*.md 2>/dev/null | wc -l)" -eq 0 ]
}

@test "with AUTOSPEC_MINE_PR_HISTORY=1: mines PRs" {
    run env AUTOSPEC_MINE_PR_HISTORY=1 bash "$SCRIPT" --repo org/repo --output-dir "$MEMORY_DIR"
    [ "$status" -eq 0 ]
    [ "$(ls "$MEMORY_DIR"/lesson_*.md 2>/dev/null | wc -l)" -ge 1 ]
}
