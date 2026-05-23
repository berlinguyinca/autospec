#!/usr/bin/env bats
# tests/mutation-adapters.bats — unit tests for per-language mutation adapters.
# Uses stubbed external tools to avoid requiring stryker/mutmut/go-mutesting installed.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    STRYKER_ADAPTER="$REPO_ROOT/mutation-adapters/stryker.sh"
    MUTMUT_ADAPTER="$REPO_ROOT/mutation-adapters/mutmut.sh"
    GOMUT_ADAPTER="$REPO_ROOT/mutation-adapters/go-mutesting.sh"

    WORK="$(mktemp -d -t mutation-adapter-test.XXXXXX)"

    # Create a minimal source fixture for each language
    printf 'function foo() { return x === "val"; }\n' > "$WORK/sample.ts"
    printf 'def check(x):\n    return x == "expected"\n' > "$WORK/sample.py"

    # Stub bin dir — only contains our stubs, no system tools.
    # For tool-absent tests: use PATH with only safe tools (no stryker/mutmut/go-mutesting/npx).
    STUB_BIN="$WORK/stub-bin"
    mkdir -p "$STUB_BIN"

    # SAFE_PATH: minimal PATH with essential tools but not mutation testing tools
    # We use the stub-bin first so stubs shadow system tools when present
    SAFE_PATH="$STUB_BIN:/usr/bin:/bin"
    export ORIGINAL_PATH="$PATH"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

# ── Helper: write a stub executable ──────────────────────────────────────────

_write_stub() {
    local name="$1"
    local body="$2"
    printf '#!/usr/bin/env bash\n%s\n' "$body" > "$STUB_BIN/$name"
    chmod +x "$STUB_BIN/$name"
}

# ── Syntax checks ─────────────────────────────────────────────────────────────

@test "stryker.sh: bash -n syntax check" {
    run bash -n "$STRYKER_ADAPTER"
    [ "$status" -eq 0 ]
}

@test "mutmut.sh: bash -n syntax check" {
    run bash -n "$MUTMUT_ADAPTER"
    [ "$status" -eq 0 ]
}

@test "go-mutesting.sh: bash -n syntax check" {
    run bash -n "$GOMUT_ADAPTER"
    [ "$status" -eq 0 ]
}

# ── stryker.sh ────────────────────────────────────────────────────────────────

@test "stryker.sh: tool-absent exits 0 with warning when npx missing" {
    # Run via a wrapper script that restricts PATH to exclude npx
    run bash -c "PATH='$SAFE_PATH' bash '$STRYKER_ADAPTER' '$WORK/sample.ts' 2>/dev/null"
    [ "$status" -eq 0 ]
}

@test "stryker.sh: tool-absent emits JSON with total:0" {
    run bash -c "PATH='$SAFE_PATH' bash '$STRYKER_ADAPTER' '$WORK/sample.ts' 2>/dev/null"
    [ "$status" -eq 0 ]
    echo "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read().strip())
assert d['total'] == 0, f'Expected total=0 but got {d}'
assert 'killed' in d, 'Missing killed key'
assert 'file' in d, 'Missing file key'
"
}

@test "stryker.sh: emits JSON matching M2 contract shape" {
    # Tool absent → still emits valid JSON with required keys
    run bash -c "PATH='$SAFE_PATH' bash '$STRYKER_ADAPTER' '$WORK/sample.ts' 2>/dev/null"
    [ "$status" -eq 0 ]
    echo "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read().strip())
for k in ['total', 'killed', 'file']:
    assert k in d, f'Missing key: {k}'
assert isinstance(d['total'], int), 'total must be int'
assert isinstance(d['killed'], int), 'killed must be int'
"
}

# ── mutmut.sh ─────────────────────────────────────────────────────────────────

@test "mutmut.sh: tool-absent exits 0 with warning when mutmut missing" {
    run bash -c "PATH='$SAFE_PATH' bash '$MUTMUT_ADAPTER' '$WORK/sample.py' 2>/dev/null"
    [ "$status" -eq 0 ]
}

@test "mutmut.sh: tool-absent emits JSON with total:0" {
    run bash -c "PATH='$SAFE_PATH' bash '$MUTMUT_ADAPTER' '$WORK/sample.py' 2>/dev/null"
    [ "$status" -eq 0 ]
    echo "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read().strip())
assert d['total'] == 0, f'Expected total=0 but got {d}'
assert 'killed' in d
assert 'file' in d
"
}

@test "mutmut.sh: parses mutmut results format when tool present" {
    # Stub mutmut to emit known results format
    _write_stub "mutmut" '
if [ "${1:-}" = "run" ]; then exit 0; fi
if [ "${1:-}" = "results" ]; then
    printf "KILLED mutants:\t5\nSURVIVED mutants:\t1\nTIMEOUT mutants:\t0\nSUSPICIOUS mutants:\t0\n"
    exit 0
fi
exit 0
'
    run env PATH="$STUB_BIN:$ORIGINAL_PATH" bash "$MUTMUT_ADAPTER" "$WORK/sample.py"
    # Survived=1 → exit 1
    [ "$status" -eq 1 ]
    echo "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read().strip())
assert d['total'] == 6, f'Expected total=6 but got {d}'
assert d['killed'] == 5, f'Expected killed=5 but got {d}'
"
}

# ── go-mutesting.sh ───────────────────────────────────────────────────────────

@test "go-mutesting.sh: tool-absent exits 0 with warning when go-mutesting missing" {
    run bash -c "PATH='$SAFE_PATH' bash '$GOMUT_ADAPTER' './...' 2>/dev/null"
    [ "$status" -eq 0 ]
}

@test "go-mutesting.sh: tool-absent emits JSON with total:0" {
    run bash -c "PATH='$SAFE_PATH' bash '$GOMUT_ADAPTER' './...' 2>/dev/null"
    [ "$status" -eq 0 ]
    echo "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read().strip())
assert d['total'] == 0, f'Expected total=0 but got {d}'
assert 'killed' in d
assert 'file' in d
"
}

@test "go-mutesting.sh: parses go-mutesting summary line when tool present" {
    # Stub go-mutesting to emit known summary
    _write_stub "go-mutesting" '
printf "The mutation score is 0.82 (9 passed, 2 failed, 0 skipped, 11 total)\n"
exit 0
'
    run env PATH="$STUB_BIN:$ORIGINAL_PATH" bash "$GOMUT_ADAPTER" "./..."
    # 2 survived → exit 1
    [ "$status" -eq 1 ]
    echo "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read().strip())
assert d['total'] == 11, f'Expected total=11 but got {d}'
assert d['killed'] == 9, f'Expected killed=9 but got {d}'
"
}

@test "go-mutesting.sh: all-killed exits 0" {
    _write_stub "go-mutesting" '
printf "The mutation score is 1.00 (5 passed, 0 failed, 0 skipped, 5 total)\n"
exit 0
'
    run env PATH="$STUB_BIN:$ORIGINAL_PATH" bash "$GOMUT_ADAPTER" "./..."
    [ "$status" -eq 0 ]
    echo "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read().strip())
assert d['total'] == 5
assert d['killed'] == 5
"
}
