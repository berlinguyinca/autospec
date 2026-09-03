#!/usr/bin/env bash
# scripts/validate-agentic-rag.sh — gate the Agentic RAG subsystem.
#
# Five checks, each guarding an invariant that a later change could break
# silently:
#
#   1. Module inventory   — every module the design document claims exists.
#   2. Specification refs  — every module cites the spec sections it implements,
#                            so a reader can get from code back to the contract.
#   3. No binary floats    — scores stay integer (architecture fitness rule
#                            financial_no_f64, and reproducible thresholds).
#   4. File size ratchet   — no rag module over the repository's 600-line cap.
#   5. Tests               — the evaluation suites of specification section 55.
#
# Usage:
#   scripts/validate-agentic-rag.sh            Run every check
#   scripts/validate-agentic-rag.sh --no-tests Skip the cargo test step
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
RAG_DIR="crates/autospec-core/src/rag"
MAX_LOC=600
RUN_TESTS=1
FAILURES=0

usage() {
    cat <<'EOF'
Usage: scripts/validate-agentic-rag.sh [--no-tests] [-h|--help]

Validate the Agentic RAG subsystem: module inventory, specification
cross-references, the no-f64 rule, the file-size ratchet, and the section 55
evaluation suites.
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-tests) RUN_TESTS=0 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

cd "$REPO_ROOT"

fail() {
    echo "ERROR: $1" >&2
    FAILURES=$((FAILURES + 1))
}

# ---------------------------------------------------------------- 1. inventory
MODULES="
authority
baseline
budget
cache
compression
config
context_package
contradiction
coordinator
evaluator
evidence
freshness
graph
injection
memory
metrics
policy
query
routing
score
scope
source
trace
"

echo "== module inventory =="
for module in $MODULES; do
    path="$RAG_DIR/$module.rs"
    if [ ! -f "$path" ]; then
        fail "missing rag module: $path"
        continue
    fi
    if ! grep -q "^pub mod $module;" "$RAG_DIR/mod.rs"; then
        fail "$module.rs exists but is not declared in $RAG_DIR/mod.rs"
    fi
done

if ! grep -q "^pub mod rag;" crates/autospec-core/src/lib.rs; then
    fail "the rag module is not declared in crates/autospec-core/src/lib.rs"
fi

# ------------------------------------------------------- 2. specification refs
# Every module must name the specification sections it implements, in its own
# header. Without this the mapping lives only in the design document, and the
# design document is the first thing to go stale.
echo "== specification cross-references =="
for module in $MODULES; do
    path="$RAG_DIR/$module.rs"
    [ -f "$path" ] || continue
    if ! head -n 12 "$path" | grep -Eqi 'spec(ification)? section'; then
        fail "$path does not cite a specification section in its header"
    fi
done

if [ ! -f docs/specs/2026-09-03-agentic-rag-subsystem-design.md ]; then
    fail "missing docs/specs/2026-09-03-agentic-rag-subsystem-design.md"
fi

# ------------------------------------------------------------- 3. no f64 rule
echo "== integer scores =="
if grep -RnE '\bf(32|64)\b' "$RAG_DIR" >/dev/null 2>&1; then
    grep -RnE '\bf(32|64)\b' "$RAG_DIR" >&2 || true
    fail "binary floating point in $RAG_DIR; scores are permille integers"
fi

# ------------------------------------------------------- 4. file-size ratchet
echo "== file size ratchet (max ${MAX_LOC} lines) =="
while IFS= read -r path; do
    lines="$(wc -l < "$path" | tr -d ' ')"
    if [ "$lines" -gt "$MAX_LOC" ]; then
        fail "$path is $lines lines, over the $MAX_LOC cap; split it"
    fi
done < <(find "$RAG_DIR" -name '*.rs' -type f)

# --------------------------------------------------------------- 5. the tests
if [ "$RUN_TESTS" -eq 1 ]; then
    echo "== evaluation suites =="
    SUITES="
rag_benchmark
rag_cache_worktree
rag_contradiction
rag_evidence
rag_graph
rag_injection
rag_policy_context
rag_retrieval_loop
rag_routing_config
rag_trace_query
"
    for suite in $SUITES; do
        if [ ! -f "crates/autospec-core/tests/$suite.rs" ]; then
            fail "missing evaluation suite: crates/autospec-core/tests/$suite.rs"
        fi
    done
    if ! cargo test -p autospec-core --lib rag:: --no-fail-fast; then
        fail "rag module unit tests failed"
    fi
    for suite in $SUITES; do
        [ -f "crates/autospec-core/tests/$suite.rs" ] || continue
        if ! cargo test -p autospec-core --test "$suite" --no-fail-fast; then
            fail "evaluation suite failed: $suite"
        fi
    done
    if ! cargo test -p autospec-cli --test rag_commands --no-fail-fast; then
        fail "autospec rag command tests failed"
    fi
else
    echo "== evaluation suites (skipped) =="
fi

if [ "$FAILURES" -gt 0 ]; then
    echo "FAIL: $FAILURES agentic-rag validation failure(s)" >&2
    exit 1
fi
echo "OK: agentic-rag validation passed"
