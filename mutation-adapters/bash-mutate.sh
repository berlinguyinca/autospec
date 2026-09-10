#!/usr/bin/env bash
# mutation-adapters/bash-mutate.sh — bash mutation testing adapter.
#
# Usage:
#   bash mutation-adapters/bash-mutate.sh <source.sh> [<bats-test-or-test-dir>]
# <bats-test-or-test-dir> is a .bats file, or a directory containing
# <basename>.bats for the source file.
#
# Applies bash-mutate.mjs to <source.sh>, applies each mutant, and runs the
# named bats test for the source file against it. Emits aggregate JSON to
# stdout: { total: N, killed: K, file: "<path>" }
#
# Every apply step is asserted (issue #3677) — a validation step that cannot
# run fails loudly, never like a pass:
#   M1 the edit must land: the applied file's hash must equal the generated
#      mutant's hash and differ from the original (MUTATION_NOT_APPLIED)
#   M2 the mutant must still parse: bash -n must pass (MUTANT_WONT_BUILD)
#   M3 the mutant must fail the NAMED test for the source file — "some test
#      failed" is weaker than "the test claiming this property failed"; an
#      unnamed whole-directory suite is refused (NAMED_TEST_NOT_FOUND)
#
# Exit codes:
#   0 — all mutants killed (full mutation coverage)
#   1 — one or more mutants survived (coverage gap)
#   2 — usage/setup error, or a validation step that could not run:
#       NAMED_TEST_NOT_FOUND, MUTATION_NOT_APPLIED, MUTANT_WONT_BUILD
#
# Environment:
#   BASH_MUTATE_MJS   path to bash-mutate.mjs (default: scripts/bash-mutate.mjs
#                     relative to this script's repo root)
#   BASH_MUTATE_WORK  scratch directory for mutants (default: tmp dir, auto-cleaned)

set -eu

# ── Argument parsing ──────────────────────────────────────────────────────────

if [ $# -lt 1 ]; then
    printf 'Usage: bash mutation-adapters/bash-mutate.sh <source.sh> [<bats-test-or-test-dir>]\n' >&2
    exit 2
fi

SOURCE_FILE="$1"
BATS_TEST_DIR="${2:-}"

if [ ! -f "$SOURCE_FILE" ]; then
    printf 'bash-mutate.sh: source file not found: %s\n' "$SOURCE_FILE" >&2
    exit 2
fi

# Locate repo root (parent of this script's directory)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Default bash-mutate.mjs location
BASH_MUTATE_MJS="${BASH_MUTATE_MJS:-$REPO_ROOT/scripts/bash-mutate.mjs}"

if [ ! -f "$BASH_MUTATE_MJS" ]; then
    printf 'bash-mutate.sh: bash-mutate.mjs not found at %s\n' "$BASH_MUTATE_MJS" >&2
    exit 2
fi

if ! command -v node >/dev/null 2>&1; then
    printf 'bash-mutate.sh: node not found in PATH\n' >&2
    exit 2
fi

if ! command -v bats >/dev/null 2>&1; then
    printf 'bash-mutate.sh: bats not found in PATH\n' >&2
    exit 2
fi

# ── Resolve the named test (M3) ───────────────────────────────────────────────
# A mutant counts as killed only when the NAMED test for the source file
# fails. Running an unnamed whole-directory suite is weaker: a mutant that
# breaks some unrelated test has not verified the property under test. When
# no named test can be resolved the gate fails loudly (exit 2) rather than
# falling back to an unnamed suite.

src_base="$(basename "$SOURCE_FILE" .sh)"
named_test=""
if [ -n "$BATS_TEST_DIR" ]; then
    if [ -f "$BATS_TEST_DIR" ]; then
        named_test="$BATS_TEST_DIR"
    elif [ -d "$BATS_TEST_DIR" ] && [ -f "$BATS_TEST_DIR/${src_base}.bats" ]; then
        named_test="$BATS_TEST_DIR/${src_base}.bats"
    fi
else
    CANDIDATE="$REPO_ROOT/tests/${src_base}.bats"
    if [ -f "$CANDIDATE" ]; then
        named_test="$CANDIDATE"
    fi
fi

if [ -z "$named_test" ]; then
    printf 'NAMED_TEST_NOT_FOUND: no named bats test for %s (looked for %s.bats) — refusing to validate against an unnamed suite\n' \
        "$SOURCE_FILE" "$src_base" >&2
    exit 2
fi

# ── Work directory setup ──────────────────────────────────────────────────────

if [ -n "${BASH_MUTATE_WORK:-}" ]; then
    WORK_DIR="$BASH_MUTATE_WORK"
    mkdir -p "$WORK_DIR"
    CLEANUP_WORK=0
else
    WORK_DIR="$(mktemp -d -t bash-mutate-XXXXXX)"
    CLEANUP_WORK=1
fi

cleanup() {
    if [ "${CLEANUP_WORK:-0}" -eq 1 ] && [ -d "${WORK_DIR:-}" ]; then
        rm -rf "$WORK_DIR"
    fi
}
trap cleanup EXIT INT TERM

EMIT_DIR="$WORK_DIR/mutants"
mkdir -p "$EMIT_DIR"

# ── Generate mutants ──────────────────────────────────────────────────────────

MUTANTS_JSON="$WORK_DIR/mutants.json"
node "$BASH_MUTATE_MJS" --file "$SOURCE_FILE" --emit "$EMIT_DIR" > "$MUTANTS_JSON" 2>/dev/null || {
    printf 'bash-mutate.sh: bash-mutate.mjs failed on %s\n' "$SOURCE_FILE" >&2
    exit 2
}

TOTAL="$(node -e "const d=JSON.parse(require('fs').readFileSync('$MUTANTS_JSON','utf8')); process.stdout.write(String(d.length));" 2>/dev/null)"
TOTAL="${TOTAL:-0}"

if [ "$TOTAL" -eq 0 ]; then
    printf '{"total":0,"killed":0,"file":"%s"}\n' "$SOURCE_FILE"
    exit 0
fi

# ── Apply each mutant and run bats ────────────────────────────────────────────

KILLED=0
SURVIVED=0

file_hash() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        cksum "$1" | awk '{print $1"-"$2}'
    fi
}

for idx in $(seq 0 $((TOTAL - 1))); do
    OUT_FILE="$(node -e "const d=JSON.parse(require('fs').readFileSync('$MUTANTS_JSON','utf8')); process.stdout.write(d[$idx].out_file);" 2>/dev/null)"
    [ -z "$OUT_FILE" ] && continue
    [ ! -f "$OUT_FILE" ] && continue

    # M1 (pre-apply): the mutant must actually change the file. A mutant that
    # is byte-identical to the source is not a mutation.
    orig_hash="$(file_hash "$SOURCE_FILE")"
    mutant_hash="$(file_hash "$OUT_FILE")"
    if [ "$mutant_hash" = "$orig_hash" ]; then
        printf 'MUTATION_NOT_APPLIED: mutant %s (%s) is byte-identical to %s — the edit did not land\n' \
            "$idx" "$(basename "$OUT_FILE")" "$SOURCE_FILE" >&2
        exit 2
    fi

    # Back up original and apply mutant
    BACKUP="$WORK_DIR/original_backup.sh"
    cp "$SOURCE_FILE" "$BACKUP"
    cp "$OUT_FILE" "$SOURCE_FILE"

    # M1 (post-apply): the applied file must be exactly the generated mutant.
    applied_hash="$(file_hash "$SOURCE_FILE")"
    if [ "$applied_hash" != "$mutant_hash" ]; then
        printf 'MUTATION_NOT_APPLIED: mutant %s did not land on %s (hash mismatch after apply)\n' \
            "$idx" "$SOURCE_FILE" >&2
        cp "$BACKUP" "$SOURCE_FILE"
        exit 2
    fi

    # M2: the mutant must still parse ("build"). A mutant that cannot even
    # parse proves nothing about the tests; fail loudly, never count it.
    if ! bash -n "$SOURCE_FILE" 2>"$WORK_DIR/bash-n.err"; then
        printf 'MUTANT_WONT_BUILD: mutant %s (%s) failed bash -n:\n' \
            "$idx" "$(basename "$OUT_FILE")" >&2
        cat "$WORK_DIR/bash-n.err" >&2
        cp "$BACKUP" "$SOURCE_FILE"
        exit 2
    fi

    # M3: killed only when the NAMED test for the source file fails.
    if ! bats "$named_test" >/dev/null 2>&1; then
        KILLED=$((KILLED + 1))
    else
        SURVIVED=$((SURVIVED + 1))
    fi

    # Restore original
    cp "$BACKUP" "$SOURCE_FILE"
done

# ── Emit result JSON ──────────────────────────────────────────────────────────

printf '{"total":%d,"killed":%d,"file":"%s"}\n' "$TOTAL" "$KILLED" "$SOURCE_FILE"

if [ "$SURVIVED" -gt 0 ]; then
    exit 1
fi
exit 0
