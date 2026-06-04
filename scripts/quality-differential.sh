#!/usr/bin/env bash
# scripts/quality-differential.sh — the boilerplate guard (spec §5 D4).
#
# Purpose
#   When a previously LLM-driven step is "determinized" (replaced by a cheap
#   deterministic text path), this harness proves whether the deterministic
#   path is GOOD ENOUGH to ship — or whether the step must keep/restore its
#   LLM path. It does so empirically, from fixtures, not from intuition.
#
# Usage
#   scripts/quality-differential.sh --step <name> --fixtures <dir>
#
#   For each immediate fixture subdir under <dir>:
#     1. run the named step's DETERMINISTIC path on the fixture's `input`
#        file, capturing stdout as the deterministic output;
#     2. evaluate that output via the fixture's own `assert.sh`, which checks
#        SIGNAL PROPERTIES (NOT string equality) — e.g. "output references
#        >=2 content tokens from the input prompt", "no canned boilerplate
#        markers". `assert.sh` is invoked as:
#            bash <fixture>/assert.sh <det-output-file> <llm-golden-file>
#        and must exit 0 (pass) / non-zero (fail).
#
#   The harness exits NON-ZERO if ANY fixture's assert fails. A non-zero exit
#   is the verdict: "this step must keep/restore its LLM path." A zero exit
#   means the deterministic path carries the same signal as the LLM golden on
#   every fixture and is safe to ship.
#
# Fixture layout (per fixture subdir)
#   input        Input file(s) for the step. For `refine-lenses` this is the
#                raw operator prompt; the deterministic lens output produced
#                FROM it is what gets asserted.
#   llm-golden   Recorded LLM output for the same input, kept for reference
#                and passed to assert.sh as $2 (assertions MAY compare signal
#                properties against it, but MUST NOT do string equality).
#   assert.sh    Signal-property checks. Receives ($1 det-output-file,
#                $2 llm-golden-file). Exit 0 pass / non-zero fail.
#
# Steps
#   synthetic       Self-test step (see run_step_synthetic). Echoes the input
#                   verbatim — fixtures decide pass/fail, so the suite ships a
#                   negative-path pair (one signal-bearing PASS fixture, one
#                   canned-string FAIL fixture).
#   refine-lenses   First real consumer. Runs the REAL deterministic refine
#                   lens path (scripts/refine-prompt.sh --lens-mode
#                   deterministic) on the fixture prompt. Per spec §7 this is
#                   EXPECTED TO FAIL the asserts (the deterministic lenses
#                   append canned boilerplate such as the literal
#                   "What happens on empty input?"), so the harness exits
#                   non-zero — that failing verdict is the documented success
#                   condition for keeping the LLM path on refine lenses.
#
# Bash rules (repo gotchas)
#   set -eu; NO RETURN traps; if/then/fi (never `[ x ] && y`) for one-sided
#   conditionals under set -e; inline cleanup only.
#
# Exit codes
#   0  every fixture passed its assert (deterministic path is safe)
#   1  >=1 fixture failed its assert (step must keep/restore LLM path)
#   2  usage / bad args / no fixtures found / unknown step

set -eu

PROG="quality-differential.sh"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
    cat <<'EOF'
Usage:
  quality-differential.sh --step <name> --fixtures <dir>

For each fixture subdir under <dir>, run the step's deterministic path on the
fixture's `input` and evaluate the output via the fixture's `assert.sh`
(signal-property checks, NOT string equality). Exit non-zero if ANY fixture
fails => verdict: "step must keep/restore its LLM path."

Fixture layout (per subdir):
  input        input file for the step
  llm-golden   recorded LLM output (reference; passed to assert.sh as $2)
  assert.sh    signal-property checks; argv: <det-output-file> <llm-golden-file>

Steps:
  synthetic       self-test step (echoes input verbatim; fixtures decide)
  refine-lenses   real deterministic refine-prompt.sh lens path (expected FAIL
                  per spec §7 — non-zero exit is the documented verdict)

Exit codes:
  0  all fixtures passed (deterministic path safe)
  1  >=1 fixture failed (keep/restore LLM path)
  2  usage / no fixtures / unknown step
EOF
}

die() {
    # die <exit-code> <message...>
    local code="$1"; shift
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit "$code"
}

# ---------------------------------------------------------------------------
# Deterministic step paths
# ---------------------------------------------------------------------------
# Each step reads an input file path ($1) and writes its deterministic output
# to stdout. Adding a new determinized step == adding one run_step_<name>.

# synthetic — self-test step. Echoes input verbatim so that the FIXTURES (via
# their assert.sh) decide pass/fail. This lets the self-test ship a
# negative-path pair without any step-specific logic.
run_step_synthetic() {
    local input="$1"
    cat "$input"
}

# refine-lenses — the real first consumer. Runs the actual deterministic refine
# lens path on the operator prompt held in the fixture's `input`. The
# deterministic output is what gets asserted (and is expected to fail).
run_step_refine_lenses() {
    local input="$1"
    local refine="$SCRIPT_DIR/refine-prompt.sh"
    if [ ! -x "$refine" ]; then
        die 2 "refine-lenses: $refine not found/executable"
    fi
    local outfile
    outfile="$(mktemp -t qd-refine.XXXXXX)"
    local artdir
    artdir="$(mktemp -d -t qd-refine-art.XXXXXX)"
    # Force the deterministic lens path; dry-run avoids any handoff. We capture
    # the refined prompt via --output. Failures are surfaced, not swallowed.
    if AUTOSPEC_REFINE_LENS_MODE=deterministic bash "$refine" \
            --from-file "$input" \
            --lens-mode deterministic \
            --rounds 4 \
            --dry-run \
            --output "$outfile" \
            --artifact-dir "$artdir" \
            --repo-root "$SCRIPT_DIR/.." >/dev/null 2>&1; then
        cat "$outfile"
        rm -rf "$outfile" "$artdir"
    else
        rm -rf "$outfile" "$artdir"
        die 2 "refine-lenses: refine-prompt.sh deterministic run failed for $input"
    fi
}

run_deterministic_step() {
    # run_deterministic_step <step-name> <input-file>  -> stdout
    local name="$1" input="$2"
    case "$name" in
        synthetic)     run_step_synthetic "$input" ;;
        refine-lenses) run_step_refine_lenses "$input" ;;
        *)             die 2 "unknown step: $name (known: synthetic, refine-lenses)" ;;
    esac
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------
STEP=""
FIXTURES=""

while [ $# -gt 0 ]; do
    case "$1" in
        --step)     [ $# -ge 2 ] || die 2 "--step requires a value"; STEP="$2"; shift 2 ;;
        --fixtures) [ $# -ge 2 ] || die 2 "--fixtures requires a value"; FIXTURES="$2"; shift 2 ;;
        -h|--help)  usage; exit 0 ;;
        *)          die 2 "unknown arg: $1" ;;
    esac
done

if [ -z "$STEP" ]; then
    die 2 "--step is required"
fi
if [ -z "$FIXTURES" ]; then
    die 2 "--fixtures is required"
fi
if [ ! -d "$FIXTURES" ]; then
    die 2 "--fixtures dir not found: $FIXTURES"
fi

# Validate the step name early (so an empty fixture dir + bad step still errors
# with the right code path rather than silently passing).
case "$STEP" in
    synthetic|refine-lenses) ;;
    *) die 2 "unknown step: $STEP (known: synthetic, refine-lenses)" ;;
esac

rc=0
fixture_count=0

for fdir in "$FIXTURES"/*/; do
    # Guard against a literal glob when no subdirs exist.
    if [ ! -d "$fdir" ]; then
        continue
    fi
    fdir="${fdir%/}"
    local_input="$fdir/input"
    local_golden="$fdir/llm-golden"
    local_assert="$fdir/assert.sh"

    if [ ! -f "$local_input" ]; then
        printf 'SKIP %s (no input file)\n' "$fdir" >&2
        continue
    fi
    if [ ! -f "$local_assert" ]; then
        printf 'SKIP %s (no assert.sh)\n' "$fdir" >&2
        continue
    fi

    fixture_count=$((fixture_count + 1))

    # Run the deterministic step, capturing stdout to a temp file.
    det_out="$(mktemp -t qd-det.XXXXXX)"
    if ! run_deterministic_step "$STEP" "$local_input" >"$det_out" 2>/dev/null; then
        printf 'FAIL %s (deterministic step errored)\n' "$fdir" >&2
        rm -f "$det_out"
        rc=1
        continue
    fi

    # llm-golden is optional; pass an empty file if absent so assert.sh sees a
    # readable $2 either way.
    golden_arg="$local_golden"
    if [ ! -f "$local_golden" ]; then
        golden_arg="$(mktemp -t qd-golden-empty.XXXXXX)"
    fi

    if bash "$local_assert" "$det_out" "$golden_arg" >&2; then
        printf 'PASS %s\n' "$fdir" >&2
    else
        printf 'FAIL %s (assert.sh signal check failed)\n' "$fdir" >&2
        rc=1
    fi

    rm -f "$det_out"
    if [ "$golden_arg" != "$local_golden" ]; then
        rm -f "$golden_arg"
    fi
done

if [ "$fixture_count" -eq 0 ]; then
    die 2 "no usable fixtures found under $FIXTURES"
fi

if [ "$rc" -ne 0 ]; then
    printf '%s: VERDICT step=%s — deterministic path BELOW BAR; keep/restore LLM path\n' "$PROG" "$STEP" >&2
else
    printf '%s: VERDICT step=%s — deterministic path OK on all %d fixtures\n' "$PROG" "$STEP" "$fixture_count" >&2
fi

exit "$rc"
