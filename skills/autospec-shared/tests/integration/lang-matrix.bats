#!/usr/bin/env bats
# lang-matrix.bats — Integration harness for autospec-shared language matrix.
#
# Runs walker.mjs against each language fixture and diffs actual output
# against the golden walker.json.
#
# Run: bats skills/autospec-shared/tests/integration/lang-matrix.bats
# (from repo root)

setup() {
    BATS_FILE_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")" && pwd)"
    REPO_ROOT="$(cd "${BATS_FILE_DIR}/../../../.." && pwd)"
    SHARED_DIR="${REPO_ROOT}/skills/autospec-shared"
    WALKER_MJS="${SHARED_DIR}/scripts/tree-sitter-walk/walker.mjs"
    MATRIX_DIR="${SHARED_DIR}/test-targets/lang-matrix"

    # Ensure node_modules are available (run npm install if needed)
    if [ ! -d "${SHARED_DIR}/node_modules" ]; then
        npm install --prefix "${SHARED_DIR}" --silent 2>/dev/null || true
    fi
}

# Helper: run walker.mjs on a list of files, returns JSON array
run_walker() {
    local lang_dir="$1"
    shift
    local files=("$@")

    node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
import { writeFileSync } from 'node:fs';
const files = ${files_json};
const results = [];
for (const f of files) {
  results.push(await walk(f));
}
process.stdout.write(JSON.stringify(results, null, 2));
EOF
}

# Helper: normalise walker JSON for comparison (strip absolute file_path → relative)
normalise_walker() {
    local json="$1"
    local lang_dir="$2"
    echo "$json" | node -e "
const d=JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
// Normalise file_path to relative (strip absolute prefix)
for(const e of d) {
  if(e.file_path) e.file_path = e.file_path.replace(/.*lang-matrix\\//, 'lang-matrix/');
}
process.stdout.write(JSON.stringify(d));
" 2>/dev/null
}

# Helper: run walker on all source files of a language and compare to golden
check_lang() {
    local lang="$1"
    shift
    local src_files=("$@")

    local lang_dir="${MATRIX_DIR}/${lang}"
    local golden_file="${lang_dir}/golden/walker.json"

    # Build absolute paths
    local abs_files=()
    for f in "${src_files[@]}"; do
        abs_files+=("${lang_dir}/${f}")
    done

    # Build JS array literal
    local files_js="["
    for f in "${abs_files[@]}"; do
        files_js+="\"${f}\","
    done
    files_js="${files_js%,}]"

    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const files = ${files_js};
const results = [];
for (const f of files) {
  results.push(await walk(f));
}
process.stdout.write(JSON.stringify(results, null, 2));
EOF
)"

    golden="$(cat "${golden_file}")"

    norm_actual="$(normalise_walker "$actual" "$lang_dir")"
    norm_golden="$(normalise_walker "$golden" "$lang_dir")"

    [ "$norm_actual" = "$norm_golden" ]
}

# ── Node / TypeScript ─────────────────────────────────────────────────────────

@test "node: walker detects typescript language" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/node/src/cli.ts');
process.stdout.write(r.language);
EOF
)"
    [ "$actual" = "typescript" ]
}

@test "node: cli avoids debug logging APIs" {
    ! grep -Eq 'console\.(log|debug|info|warn|error)|(^|[^[:alnum:]_])debugger([^[:alnum:]_]|$)' \
        "${MATRIX_DIR}/node/src/cli.ts"
}

@test "node: walker detects cli_command entry_point in cli.ts" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/node/src/cli.ts');
const ep = r.entry_points.find(e => e.kind === 'cli_command');
process.stdout.write(ep ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "node: walker detects http_route entry_point in server.ts" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/node/src/server.ts');
const ep = r.entry_points.find(e => e.kind === 'http_route');
process.stdout.write(ep ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "node: walker detects exported function greet in lib.ts" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/node/src/lib.ts');
const exp = r.exports.find(e => e.name === 'greet');
process.stdout.write(exp ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "node: actual walker.json byte-equal to golden" {
    check_lang "node" "src/cli.ts" "src/server.ts" "src/lib.ts"
}

# ── Python ────────────────────────────────────────────────────────────────────

@test "python: walker detects python language" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/python/src/cli.py');
process.stdout.write(r.language);
EOF
)"
    [ "$actual" = "python" ]
}

@test "python: walker detects cli_command entry_point in cli.py" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/python/src/cli.py');
const ep = r.entry_points.find(e => e.kind === 'cli_command');
process.stdout.write(ep ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "python: walker detects http_route entry_point in server.py" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/python/src/server.py');
const ep = r.entry_points.find(e => e.kind === 'http_route');
process.stdout.write(ep ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "python: walker detects exported function greet in lib.py" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/python/src/lib.py');
const exp = r.exports.find(e => e.name === 'greet');
process.stdout.write(exp ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "python: actual walker.json byte-equal to golden" {
    check_lang "python" "src/cli.py" "src/server.py" "src/lib.py"
}

# ── Go ────────────────────────────────────────────────────────────────────────

@test "go: walker detects go language" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/go/cmd/cli/main.go');
process.stdout.write(r.language);
EOF
)"
    [ "$actual" = "go" ]
}

@test "go: walker detects cli_command entry_point in main.go" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/go/cmd/cli/main.go');
const ep = r.entry_points.find(e => e.kind === 'cli_command');
process.stdout.write(ep ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "go: walker detects http_route entry_point in server.go" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/go/internal/server.go');
const ep = r.entry_points.find(e => e.kind === 'http_route');
process.stdout.write(ep ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "go: walker detects exported function StartServer in server.go" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/go/internal/server.go');
const exp = r.exports.find(e => e.name === 'StartServer');
process.stdout.write(exp ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "go: actual walker.json byte-equal to golden" {
    check_lang "go" "cmd/cli/main.go" "internal/server.go"
}

# ── Rust ──────────────────────────────────────────────────────────────────────

@test "rust: walker detects rust language" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/rust/src/main.rs');
process.stdout.write(r.language);
EOF
)"
    [ "$actual" = "rust" ]
}

@test "rust: walker detects cli_command entry_point in main.rs" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/rust/src/main.rs');
const ep = r.entry_points.find(e => e.kind === 'cli_command');
process.stdout.write(ep ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "rust: walker detects http_route entry_point in server.rs" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/rust/src/server.rs');
const ep = r.entry_points.find(e => e.kind === 'http_route');
process.stdout.write(ep ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "rust: walker detects exported function greet in lib.rs" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/rust/src/lib.rs');
const exp = r.exports.find(e => e.name === 'greet');
process.stdout.write(exp ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "rust: actual walker.json byte-equal to golden" {
    check_lang "rust" "src/main.rs" "src/server.rs" "src/lib.rs"
}

# ── JVM / Java ────────────────────────────────────────────────────────────────

@test "jvm: walker detects java language" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/jvm/src/main/java/com/example/Cli.java');
process.stdout.write(r.language);
EOF
)"
    [ "$actual" = "java" ]
}

@test "jvm: walker detects cli_command entry_point in Cli.java" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/jvm/src/main/java/com/example/Cli.java');
const ep = r.entry_points.find(e => e.kind === 'cli_command');
process.stdout.write(ep ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "jvm: walker detects http_route entry_point in Server.java" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/jvm/src/main/java/com/example/Server.java');
const ep = r.entry_points.find(e => e.kind === 'http_route');
process.stdout.write(ep ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "jvm: walker detects exported function greet in Lib.java" {
    actual="$(node --input-type=module <<EOF 2>/dev/null
import { walk } from '${WALKER_MJS}';
const r = await walk('${MATRIX_DIR}/jvm/src/main/java/com/example/Lib.java');
const exp = r.exports.find(e => e.name === 'greet');
process.stdout.write(exp ? 'found' : 'missing');
EOF
)"
    [ "$actual" = "found" ]
}

@test "jvm: actual walker.json byte-equal to golden" {
    check_lang "jvm" \
        "src/main/java/com/example/Cli.java" \
        "src/main/java/com/example/Server.java" \
        "src/main/java/com/example/Lib.java"
}
