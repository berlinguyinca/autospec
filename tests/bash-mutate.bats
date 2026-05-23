#!/usr/bin/env bats
# tests/bash-mutate.bats — unit and integration tests for bash-mutate.mjs
# and mutation-adapters/bash-mutate.sh.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    MJS="$REPO_ROOT/scripts/bash-mutate.mjs"
    ADAPTER="$REPO_ROOT/mutation-adapters/bash-mutate.sh"
    FIX="$REPO_ROOT/tests/fixtures/bash-mutate"
    SAMPLE="$FIX/sample.sh"
    WORK="$(mktemp -d -t bash-mutate-test.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

# ── Syntax / invocation checks ────────────────────────────────────────────────

@test "bash-mutate.mjs: node syntax check" {
    run node --check "$MJS"
    [ "$status" -eq 0 ]
}

@test "bash-mutate.mjs: --help exits 0" {
    run node "$MJS" --help
    [ "$status" -eq 0 ]
}

@test "bash-mutate.sh: bash -n syntax check" {
    run bash -n "$ADAPTER"
    [ "$status" -eq 0 ]
}

# ── OP_FLIP_EQ operator ───────────────────────────────────────────────────────

@test "OP_FLIP_EQ: emits mutant for line with == in [ ] test" {
    # Write a minimal bash file with a == comparison
    printf '#!/usr/bin/env bash\nif [ "$x" == "val" ]; then echo ok; fi\n' \
        > "$WORK/flip_eq.sh"
    run node "$MJS" --file "$WORK/flip_eq.sh" --emit "$WORK/emit"
    [ "$status" -eq 0 ]
    # At least one OP_FLIP_EQ mutant must be in the JSON output
    echo "$output" | grep -q "OP_FLIP_EQ"
}

@test "OP_FLIP_EQ: flips == to != in mutant line" {
    printf '#!/usr/bin/env bash\nif [ "$x" == "val" ]; then echo ok; fi\n' \
        > "$WORK/flip_eq2.sh"
    run node "$MJS" --file "$WORK/flip_eq2.sh" --emit "$WORK/emit2"
    [ "$status" -eq 0 ]
    # The mutant field in JSON should contain != in the line
    echo "$output" | python3 -c "
import sys, json
data = json.load(sys.stdin)
flips = [m for m in data if m['operator'] == 'OP_FLIP_EQ']
assert any('!=' in m['mutant'] for m in flips), 'Expected != in mutant line'
"
}

@test "OP_FLIP_EQ: flips != to == in mutant line" {
    printf '#!/usr/bin/env bash\nif [ "$x" != "val" ]; then echo ok; fi\n' \
        > "$WORK/flip_ne.sh"
    run node "$MJS" --file "$WORK/flip_ne.sh" --emit "$WORK/emit_ne"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "OP_FLIP_EQ"
    echo "$output" | python3 -c "
import sys, json
data = json.load(sys.stdin)
flips = [m for m in data if m['operator'] == 'OP_FLIP_EQ']
assert any('==' in m['mutant'] for m in flips), 'Expected == in mutant line'
"
}

# ── OP_DROP_ASSERT operator ───────────────────────────────────────────────────

@test "OP_DROP_ASSERT: emits mutant for 'run' line" {
    printf '#!/usr/bin/env bash\nrun my_command\n' > "$WORK/drop_run.sh"
    run node "$MJS" --file "$WORK/drop_run.sh" --emit "$WORK/emit_run"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "OP_DROP_ASSERT"
}

@test "OP_DROP_ASSERT: emits mutant for 'grep -q' line" {
    printf '#!/usr/bin/env bash\ngrep -q "pattern" file.txt\n' > "$WORK/drop_grep.sh"
    run node "$MJS" --file "$WORK/drop_grep.sh" --emit "$WORK/emit_grep"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "OP_DROP_ASSERT"
}

@test "OP_DROP_ASSERT: mutant replaces assertion with no-op colon" {
    printf '#!/usr/bin/env bash\nrun my_command\n' > "$WORK/drop_noop.sh"
    run node "$MJS" --file "$WORK/drop_noop.sh" --emit "$WORK/emit_noop"
    [ "$status" -eq 0 ]
    # The mutant content in JSON should contain the no-op marker
    echo "$output" | grep -q "drop-assert"
}

# ── OP_SWAP_LITERAL operator ──────────────────────────────────────────────────

@test "OP_SWAP_LITERAL: emits mutant for line with string literal" {
    printf '#!/usr/bin/env bash\necho "hello world"\n' > "$WORK/swap_lit.sh"
    run node "$MJS" --file "$WORK/swap_lit.sh" --emit "$WORK/emit_lit"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "OP_SWAP_LITERAL"
}

@test "OP_SWAP_LITERAL: mutant swaps literal to empty string" {
    printf '#!/usr/bin/env bash\necho "hello"\n' > "$WORK/swap_empty.sh"
    run node "$MJS" --file "$WORK/swap_empty.sh" --emit "$WORK/emit_empty"
    [ "$status" -eq 0 ]
    # mutant field shows empty string swap
    echo "$output" | python3 -c "
import sys, json
data = json.load(sys.stdin)
op_swap = [m for m in data if m['operator'] == 'OP_SWAP_LITERAL']
assert len(op_swap) > 0, 'No OP_SWAP_LITERAL mutants'
assert '\"\"' in op_swap[0]['mutant'], 'Mutant should contain empty string'
"
}

# ── Output JSON shape ─────────────────────────────────────────────────────────

@test "bash-mutate.mjs: output is valid JSON array" {
    run node "$MJS" --file "$SAMPLE" --emit "$WORK/emit_sample"
    [ "$status" -eq 0 ]
    echo "$output" | python3 -c "import sys,json; d=json.load(sys.stdin); assert isinstance(d, list)"
}

@test "bash-mutate.mjs: each mutant has required fields" {
    run node "$MJS" --file "$SAMPLE" --emit "$WORK/emit_fields"
    [ "$status" -eq 0 ]
    echo "$output" | python3 -c "
import sys, json
data = json.load(sys.stdin)
assert len(data) > 0, 'No mutants generated'
for m in data:
    for field in ['id', 'operator', 'file', 'line', 'original', 'mutant', 'out_file']:
        assert field in m, f'Missing field: {field}'
"
}

@test "bash-mutate.mjs: generates at least 3 mutants for sample fixture" {
    run node "$MJS" --file "$SAMPLE" --emit "$WORK/emit_3"
    [ "$status" -eq 0 ]
    COUNT="$(echo "$output" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))")"
    [ "$COUNT" -ge 3 ]
}

@test "bash-mutate.mjs: all 3 operator types represented in sample output" {
    run node "$MJS" --file "$SAMPLE" --emit "$WORK/emit_ops"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "OP_FLIP_EQ"
    echo "$output" | grep -q "OP_DROP_ASSERT"
    echo "$output" | grep -q "OP_SWAP_LITERAL"
}

@test "bash-mutate.mjs: mutant files exist on disk" {
    run node "$MJS" --file "$SAMPLE" --emit "$WORK/emit_disk"
    [ "$status" -eq 0 ]
    FIRST_FILE="$(echo "$output" | python3 -c "import sys,json; print(json.load(sys.stdin)[0]['out_file'])")"
    [ -f "$FIRST_FILE" ]
}

# ── End-to-end: vacuous test survives mutant ──────────────────────────────────

@test "bash-mutate.sh: mutant survives against vacuous test (exit 1)" {
    # Use a copy of sample.sh to avoid clobbering the fixture
    cp "$SAMPLE" "$WORK/sample_vacuous.sh"
    BASH_MUTATE_WORK="$WORK/adapter_work_vacuous" \
        run bash "$ADAPTER" "$WORK/sample_vacuous.sh" "$FIX/vacuous-test.bats"
    # Exit 1 = mutants survived (coverage gap detected)
    [ "$status" -eq 1 ]
    # Output JSON must have total > 0 and killed < total
    echo "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read().strip())
assert d['total'] > 0, 'Expected mutants'
assert d['killed'] < d['total'], 'Expected some survivors'
"
}

# ── End-to-end: real test kills mutant ───────────────────────────────────────

@test "bash-mutate.sh: emits JSON with total and killed keys" {
    cp "$SAMPLE" "$WORK/sample_real.sh"
    BASH_MUTATE_WORK="$WORK/adapter_work_real" \
        run bash "$ADAPTER" "$WORK/sample_real.sh" "$FIX/real-test.bats"
    # Exit 0 = all mutants killed, exit 1 = some survived — both are valid
    # We only check the JSON shape
    echo "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read().strip())
assert 'total' in d, 'Missing total key'
assert 'killed' in d, 'Missing killed key'
assert 'file' in d, 'Missing file key'
assert d['total'] > 0, 'Expected mutants'
"
}
