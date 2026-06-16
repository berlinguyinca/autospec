#!/usr/bin/env bats
# tests/explore/test_explore_qa_gate_runner.bats — Issue #1113.
#
# scripts/explore-qa-gate.sh stands up the sandbox-HEAD app in a fresh
# worktree, runs `autospec-qa --no-heal`, and persists
# .autospec/explore-qa-gate.json per the pinned schema.
#
# Real shell, no mocks beyond a stub `autospec-qa` on PATH and a stub
# worktree-guard create seam. bash 3.2 safe.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    GATE="$REPO_ROOT/scripts/explore-qa-gate.sh"
    SCHEMA="$REPO_ROOT/schemas/autospec-explore-qa-gate.schema.json"

    TMP="$(mktemp -d -t explore-qag.XXXXXX)"
    # Primary checkout (sandbox source) — a real git repo with a commit.
    PRIMARY="$TMP/primary"
    mkdir -p "$PRIMARY"
    cd "$PRIMARY"
    git init -q
    git config user.email t@t.t
    git config user.name t
    git checkout -q -b sandbox-branch
    echo "app" > app.txt
    git add app.txt
    git commit -q -m "seed"
    HEAD_SHA="$(git rev-parse HEAD)"

    # Isolated state dir so we never touch the real ~/.autospec.
    export AUTOSPEC_STATE_DIR="$TMP/state"
    mkdir -p "$AUTOSPEC_STATE_DIR"

    # bin/ on PATH for stubs (autospec-qa, worktree-guard seam markers).
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin"

    # Worktree target the runner will create; keep it inside TMP.
    export AUTOSPEC_QA_GATE_WORKTREE="$TMP/wt-sandbox"
}

teardown() {
    rm -rf "$TMP"
}

# Seed .autospec/explore-mode.json in the primary checkout.
seed_mode() {
    mkdir -p "$PRIMARY/.autospec"
    cat > "$PRIMARY/.autospec/explore-mode.json" <<EOF
{
  "branch": "sandbox-branch",
  "slug": "demo",
  "base": "main",
  "head_sha": "$HEAD_SHA",
  "created_at": "2026-06-16T00:00:00Z"
}
EOF
}

# Stub autospec-qa: on --no-heal, write a qa-verdict.json with the verdict
# captured in $QA_STUB_VERDICT into the cwd's .autospec/.
make_qa_stub() {
    local verdict="$1"
    cat > "$TMP/bin/autospec-qa" <<EOF
#!/usr/bin/env bash
mkdir -p .autospec
cat > .autospec/qa-verdict.json <<JSON
{
  "verdict": "$verdict",
  "head_sha": "$HEAD_SHA",
  "generated_at": "2026-06-16T00:00:00Z",
  "findings": [
    { "category": "other", "release_blocking": true, "status": "$verdict", "summary": "stub finding" }
  ]
}
JSON
exit 0
EOF
    chmod +x "$TMP/bin/autospec-qa"
}

# Stub autospec-qa that crashes (non-zero, no verdict file).
make_qa_crash_stub() {
    cat > "$TMP/bin/autospec-qa" <<'EOF'
#!/usr/bin/env bash
echo "boom" >&2
exit 7
EOF
    chmod +x "$TMP/bin/autospec-qa"
}

# Mark a QA config so the probe does not SKIP.
add_qa_config() {
    mkdir -p "$PRIMARY/.autospec"
    printf 'spec_file: spec.md\n' > "$PRIMARY/.autospec/test.yml"
}

# ---------------------------------------------------------------------------
# Schema sanity
# ---------------------------------------------------------------------------

@test "schema file exists and is valid JSON" {
    [ -f "$SCHEMA" ]
    run jq -e . "$SCHEMA"
    [ "$status" -eq 0 ]
}

@test "bash -n parses the runner" {
    run bash -n "$GATE"
    [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------------------
# No sandbox -> fail-closed operator error
# ---------------------------------------------------------------------------

@test "missing explore-mode.json -> non-zero + explore_qa_gate_no_sandbox" {
    cd "$PRIMARY"
    run bash "$GATE"
    [ "$status" -ne 0 ]
    echo "$output" | grep -q 'code_health:explore_qa_gate_no_sandbox'
}

# ---------------------------------------------------------------------------
# No QA config -> SKIP (fail-open)
# ---------------------------------------------------------------------------

@test "no QA config -> verdict skipped, exit 0, skip code_health" {
    cd "$PRIMARY"
    seed_mode
    make_qa_stub PASS   # present but must not be invoked; probe skips first
    run bash "$GATE"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'code_health:explore_qa_gate_skipped_no_config'
    run jq -r '.verdict' "$PRIMARY/.autospec/explore-qa-gate.json"
    [ "$output" = "skipped" ]
}

# ---------------------------------------------------------------------------
# PASS / FAIL / PARTIAL passthrough
# ---------------------------------------------------------------------------

@test "QA PASS -> explore-qa-gate.json verdict PASS, exit 0" {
    cd "$PRIMARY"
    seed_mode
    add_qa_config
    make_qa_stub PASS
    run bash "$GATE"
    [ "$status" -eq 0 ]
    run jq -r '.verdict' "$PRIMARY/.autospec/explore-qa-gate.json"
    [ "$output" = "PASS" ]
}

@test "QA FAIL -> verdict FAIL and non-zero block" {
    cd "$PRIMARY"
    seed_mode
    add_qa_config
    make_qa_stub FAIL
    run bash "$GATE"
    [ "$status" -ne 0 ]
    run jq -r '.verdict' "$PRIMARY/.autospec/explore-qa-gate.json"
    [ "$output" = "FAIL" ]
}

@test "QA PARTIAL -> verdict PARTIAL and non-zero block" {
    cd "$PRIMARY"
    seed_mode
    add_qa_config
    make_qa_stub PARTIAL
    run bash "$GATE"
    [ "$status" -ne 0 ]
    run jq -r '.verdict' "$PRIMARY/.autospec/explore-qa-gate.json"
    [ "$output" = "PARTIAL" ]
}

# ---------------------------------------------------------------------------
# QA crash -> error (fail-closed block)
# ---------------------------------------------------------------------------

@test "QA crash -> verdict error and non-zero block" {
    cd "$PRIMARY"
    seed_mode
    add_qa_config
    make_qa_crash_stub
    run bash "$GATE"
    [ "$status" -ne 0 ]
    run jq -r '.verdict' "$PRIMARY/.autospec/explore-qa-gate.json"
    [ "$output" = "error" ]
}

@test "missing autospec-qa on PATH -> verdict error and non-zero block" {
    cd "$PRIMARY"
    seed_mode
    add_qa_config
    # no autospec-qa stub created; ensure none resolves
    rm -f "$TMP/bin/autospec-qa"
    run bash "$GATE"
    [ "$status" -ne 0 ]
    run jq -r '.verdict' "$PRIMARY/.autospec/explore-qa-gate.json"
    [ "$output" = "error" ]
}

# ---------------------------------------------------------------------------
# Reads sandbox HEAD from explore-mode.json; records it
# ---------------------------------------------------------------------------

@test "records sandbox_branch and sandbox_head_sha from explore-mode.json" {
    cd "$PRIMARY"
    seed_mode
    add_qa_config
    make_qa_stub PASS
    run bash "$GATE"
    [ "$status" -eq 0 ]
    run jq -r '.sandbox_head_sha' "$PRIMARY/.autospec/explore-qa-gate.json"
    [ "$output" = "$HEAD_SHA" ]
    run jq -r '.sandbox_branch' "$PRIMARY/.autospec/explore-qa-gate.json"
    [ "$output" = "sandbox-branch" ]
}

# ---------------------------------------------------------------------------
# Worktree isolation: primary checkout must not be edited by the gate
# ---------------------------------------------------------------------------

@test "primary checkout stays clean (gate ran QA in a worktree, not here)" {
    cd "$PRIMARY"
    seed_mode
    add_qa_config
    make_qa_stub PASS
    run bash "$GATE"
    [ "$status" -eq 0 ]
    # The only artifact written in the primary checkout is the gate JSON +
    # the seeded mode/config. The stub's qa-verdict.json must land in the
    # worktree, NOT in the primary .autospec/. Assert no qa-verdict.json here.
    [ ! -f "$PRIMARY/.autospec/qa-verdict.json" ]
}

# ---------------------------------------------------------------------------
# Output validates against the schema
# ---------------------------------------------------------------------------

@test "explore-qa-gate.json validates against schema (ajv if available)" {
    cd "$PRIMARY"
    seed_mode
    add_qa_config
    make_qa_stub PASS
    run bash "$GATE"
    [ "$status" -eq 0 ]
    if command -v ajv >/dev/null 2>&1; then
        run ajv validate --spec=draft2020 -s "$SCHEMA" -d "$PRIMARY/.autospec/explore-qa-gate.json"
        [ "$status" -eq 0 ]
    else
        # Minimal structural check: all required keys present.
        run jq -e 'has("verdict") and has("sandbox_branch") and has("sandbox_head_sha") and has("qa_verdict_path") and has("blocking_findings") and has("ran_at")' \
            "$PRIMARY/.autospec/explore-qa-gate.json"
        [ "$status" -eq 0 ]
    fi
}
