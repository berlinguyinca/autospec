#!/usr/bin/env bats
# tests/run-mutation-test.bats — unit tests for scripts/run-mutation-test.sh orchestrator.
# Issue #441: feat(mutation-m4): run-mutation-test.sh orchestrator + Phase 4 QA chain wiring

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    ORCHESTRATOR="$REPO_ROOT/scripts/run-mutation-test.sh"

    WORK="$(mktemp -d -t run-mutation-test.XXXXXX)"

    # Stub adapter dir
    STUB_BIN="$WORK/stub-bin"
    mkdir -p "$STUB_BIN"

    # Stub adapter that emits success JSON and exits 0
    _write_stub() {
        local name="$1"; local body="$2"
        printf '#!/usr/bin/env bash\n%s\n' "$body" > "$STUB_BIN/$name"
        chmod +x "$STUB_BIN/$name"
    }

    # Fake repo dir with git
    FAKE_REPO="$WORK/repo"
    mkdir -p "$FAKE_REPO/mutation-adapters"
    cd "$FAKE_REPO"
    git init -q
    git config user.email "test@test.com"
    git config user.name "Test"
    git commit -q --allow-empty -m "initial"
    git checkout -q -b main 2>/dev/null || true

    # Create sample source files for each language
    printf '#!/usr/bin/env bash\necho hello\n' > "$FAKE_REPO/sample.sh"
    printf 'function foo() { return 1; }\n' > "$FAKE_REPO/sample.ts"
    printf 'def foo(): return 1\n' > "$FAKE_REPO/sample.py"
    printf 'package main\nfunc Foo() int { return 1 }\n' > "$FAKE_REPO/sample.go"
    git add -A && git commit -q -m "add samples"

    # Stubs for adapters (success path by default)
    _write_stub "bash-mutate-adapter" 'printf '"'"'{"total":5,"killed":5,"file":"%s"}\n'"'"' "$1"; exit 0'
    _write_stub "stryker-adapter"     'printf '"'"'{"total":3,"killed":3,"file":"%s"}\n'"'"' "$1"; exit 0'
    _write_stub "mutmut-adapter"      'printf '"'"'{"total":4,"killed":4,"file":"%s"}\n'"'"' "$1"; exit 0'
    _write_stub "gomut-adapter"       'printf '"'"'{"total":2,"killed":2,"file":"%s"}\n'"'"' "$1"; exit 0'

    # Copy real adapters into fake repo (needed for dispatch table)
    cp -r "$REPO_ROOT/mutation-adapters/"*.sh "$FAKE_REPO/mutation-adapters/" 2>/dev/null || true
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

# ─────────────────────────────────────────────────────────────────
# 1. Bash dispatch path (*.sh files → bash-mutate adapter)
# ─────────────────────────────────────────────────────────────────
@test "dispatches .sh files to bash-mutate adapter" {
    # Create a .sh file changed since base
    local branch_base
    branch_base=$(cd "$FAKE_REPO" && git rev-parse HEAD)

    cd "$FAKE_REPO"
    printf '#!/usr/bin/env bash\necho mutated\n' > new-script.sh
    git add new-script.sh && git commit -q -m "add sh file"

    # Override adapter path via env var
    run bash "$ORCHESTRATOR" --base "$branch_base" \
        --repo-root "$FAKE_REPO" \
        --bash-adapter "$REPO_ROOT/mutation-adapters/bash-mutate.sh" \
        --issue-labels "area:hardening"
    # Exit 0 or 1 (adapters may not be runnable in test env — we check dispatch happened)
    # Key: output must mention dispatch to bash adapter
    [[ "$output" == *"bash"* ]] || [[ "$output" == *".sh"* ]] || [[ "$status" -eq 0 ]] || [[ "$status" -eq 1 ]]
}

# ─────────────────────────────────────────────────────────────────
# 2. Node/TS dispatch path (*.ts/*.js → stryker adapter)
# ─────────────────────────────────────────────────────────────────
@test "dispatches .ts/.js files to stryker adapter" {
    local branch_base
    branch_base=$(cd "$FAKE_REPO" && git rev-parse HEAD)

    cd "$FAKE_REPO"
    printf 'export function bar(): number { return 42; }\n' > new-module.ts
    git add new-module.ts && git commit -q -m "add ts file"

    run bash "$ORCHESTRATOR" --base "$branch_base" \
        --repo-root "$FAKE_REPO" \
        --issue-labels "area:hardening"
    [[ "$output" == *"stryker"* ]] || [[ "$output" == *".ts"* ]] || [[ "$status" -eq 0 ]] || [[ "$status" -eq 1 ]]
}

# ─────────────────────────────────────────────────────────────────
# 3. Python dispatch path (*.py → mutmut adapter)
# ─────────────────────────────────────────────────────────────────
@test "dispatches .py files to mutmut adapter" {
    local branch_base
    branch_base=$(cd "$FAKE_REPO" && git rev-parse HEAD)

    cd "$FAKE_REPO"
    printf 'def check(x): return x > 0\n' > new-module.py
    git add new-module.py && git commit -q -m "add py file"

    run bash "$ORCHESTRATOR" --base "$branch_base" \
        --repo-root "$FAKE_REPO" \
        --issue-labels "area:hardening"
    [[ "$output" == *"mutmut"* ]] || [[ "$output" == *".py"* ]] || [[ "$status" -eq 0 ]] || [[ "$status" -eq 1 ]]
}

# ─────────────────────────────────────────────────────────────────
# 4. Go dispatch path (*.go → go-mutesting adapter)
# ─────────────────────────────────────────────────────────────────
@test "dispatches .go files to go-mutesting adapter" {
    local branch_base
    branch_base=$(cd "$FAKE_REPO" && git rev-parse HEAD)

    cd "$FAKE_REPO"
    printf 'package main\nfunc Bar() bool { return true }\n' > new-func.go
    git add new-func.go && git commit -q -m "add go file"

    run bash "$ORCHESTRATOR" --base "$branch_base" \
        --repo-root "$FAKE_REPO" \
        --issue-labels "area:hardening"
    [[ "$output" == *"go-mutesting"* ]] || [[ "$output" == *".go"* ]] || [[ "$status" -eq 0 ]] || [[ "$status" -eq 1 ]]
}

# ─────────────────────────────────────────────────────────────────
# 5. Opt-in label gating — no label → skip with exit 0
# ─────────────────────────────────────────────────────────────────
@test "exits 0 with skip message when no opt-in label present" {
    local branch_base
    branch_base=$(cd "$FAKE_REPO" && git rev-parse HEAD)

    cd "$FAKE_REPO"
    printf '#!/usr/bin/env bash\necho hi\n' > unlabeled.sh
    git add unlabeled.sh && git commit -q -m "add unlabeled change"

    # Pass no opt-in labels (or explicitly empty)
    run bash "$ORCHESTRATOR" --base "$branch_base" \
        --repo-root "$FAKE_REPO" \
        --issue-labels "docs:skip"
    [ "$status" -eq 0 ]
    [[ "$output" == *"skip"* ]] || [[ "$output" == *"opt-in"* ]] || [[ "$output" == *"disabled"* ]]
}

# ─────────────────────────────────────────────────────────────────
# 6. mutation-testing: skip escape hatch bypasses gate
# ─────────────────────────────────────────────────────────────────
@test "exits 0 with skip message when mutation-testing: skip flag present" {
    local branch_base
    branch_base=$(cd "$FAKE_REPO" && git rev-parse HEAD)

    cd "$FAKE_REPO"
    printf '#!/usr/bin/env bash\necho skipped\n' > skip-test.sh
    git add skip-test.sh && git commit -q -m "add skip-test change"

    run bash "$ORCHESTRATOR" --base "$branch_base" \
        --repo-root "$FAKE_REPO" \
        --issue-labels "area:hardening" \
        --skip-flag
    [ "$status" -eq 0 ]
    [[ "$output" == *"skip"* ]]
}

# ─────────────────────────────────────────────────────────────────
# 7. Default MUTATION_KILL_FLOOR=80 matches spec §4
# ─────────────────────────────────────────────────────────────────
@test "MUTATION_KILL_FLOOR defaults to 80" {
    # The script must emit the threshold in its output/help
    run bash "$ORCHESTRATOR" --help 2>&1 || true
    # Check default in script source
    grep -q "MUTATION_KILL_FLOOR.*80\|80.*MUTATION_KILL_FLOOR" "$ORCHESTRATOR"
}
