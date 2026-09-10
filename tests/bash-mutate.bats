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

# ── Apply assertions (issue #3677): a validation step that cannot run ─────────
# must fail loudly, never look like a pass.

_write_fake_mjs() {
    # $1 = path to fake mjs, $2 = body of the mutants array literal
    cat > "$1" <<FAKE_MJS
import fs from 'node:fs';
const args = process.argv.slice(2);
let file = '', emit = '';
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--file' && args[i + 1]) { file = args[++i]; }
  else if (args[i] === '--emit' && args[i + 1]) { emit = args[++i]; }
}
fs.mkdirSync(emit, { recursive: true });
const out = emit + '/fake.mut1.OP_TEST.sh';
fs.writeFileSync(out, $2);
process.stdout.write(JSON.stringify([
  { id: 1, operator: 'OP_TEST', file: file, line: 1, original: 'x', mutant: 'y', out_file: out }
]) + '\n');
FAKE_MJS
}

@test "bash-mutate.sh: MUTATION_NOT_APPLIED when mutant is byte-identical to source (exit 2)" {
    printf '#!/usr/bin/env bash\nif [ "$x" == "v" ]; then echo ok; fi\n' > "$WORK/ident.sh"
    _write_fake_mjs "$WORK/ident-mutate.mjs" "fs.readFileSync(file, 'utf8')"
    BASH_MUTATE_MJS="$WORK/ident-mutate.mjs" \
        BASH_MUTATE_WORK="$WORK/adapter_work_ident" \
        run bash "$ADAPTER" "$WORK/ident.sh" "$FIX/vacuous-test.bats"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "MUTATION_NOT_APPLIED"
    # Source must be untouched
    grep -q '== "v"' "$WORK/ident.sh"
}

@test "bash-mutate.sh: MUTANT_WONT_BUILD when mutant fails bash -n (exit 2)" {
    printf '#!/usr/bin/env bash\nif [ "$x" == "v" ]; then echo ok; fi\n' > "$WORK/broken.sh"
    _write_fake_mjs "$WORK/broken-mutate.mjs" "'#!/usr/bin/env bash\\nif [ 1; then\\n'"
    BASH_MUTATE_MJS="$WORK/broken-mutate.mjs" \
        BASH_MUTATE_WORK="$WORK/adapter_work_broken" \
        run bash "$ADAPTER" "$WORK/broken.sh" "$FIX/vacuous-test.bats"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "MUTANT_WONT_BUILD"
    # Source must be restored after the failed apply
    grep -q '== "v"' "$WORK/broken.sh"
}

@test "bash-mutate.sh: NAMED_TEST_NOT_FOUND when no named bats file exists (exit 2)" {
    printf '#!/usr/bin/env bash\nif [ "$x" == "v" ]; then echo ok; fi\n' > "$WORK/zz_no_named_test_3677.sh"
    run bash "$ADAPTER" "$WORK/zz_no_named_test_3677.sh"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "NAMED_TEST_NOT_FOUND"
}

@test "bash-mutate.sh: NAMED_TEST_NOT_FOUND when test dir lacks <basename>.bats (exit 2)" {
    cp "$SAMPLE" "$WORK/named_dir.sh"
    mkdir -p "$WORK/empty_tests"
    BASH_MUTATE_WORK="$WORK/adapter_work_named_dir" \
        run bash "$ADAPTER" "$WORK/named_dir.sh" "$WORK/empty_tests"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "NAMED_TEST_NOT_FOUND"
}

@test "bash-mutate.sh: named test kills all mutants (exit 0, killed == total)" {
    # A source whose single property is claimed by exactly one named test.
    {
        printf '#!/usr/bin/env bash\n'
        printf 'check_flag() {\n'
        printf '    if [ "${FLAG:-off}" == "on" ]; then\n'
        printf '        echo "enabled"\n'
        printf '    fi\n'
        printf '}\n'
    } > "$WORK/killme.sh"
    {
        printf '#!/usr/bin/env bats\n'
        printf '@test "check_flag: enabled when FLAG=on" {\n'
        printf '    FLAG=on\n'
        printf '    source "$BATS_TEST_DIRNAME/killme.sh"\n'
        printf '    run check_flag\n'
        printf '    [ "$status" -eq 0 ]\n'
        printf '    [ "$output" = "enabled" ]\n'
        printf '}\n'
    } > "$WORK/killme.bats"
    BASH_MUTATE_WORK="$WORK/adapter_work_killme" \
        run bash "$ADAPTER" "$WORK/killme.sh" "$WORK"
    [ "$status" -eq 0 ]
    echo "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read().strip())
assert d['total'] > 0, 'Expected mutants'
assert d['killed'] == d['total'], 'Expected every mutant killed by the named test'
"
}
