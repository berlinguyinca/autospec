#!/usr/bin/env bats
# define-fab-lens.bats — the autospec-define fab decomposition lens (Part A) +
# an end-to-end dogfood that proves define→run→green release-gate (Part B).
#
# Two concerns:
#   1. The fab lens prose is present in skills/autospec-define/SKILL.md:
#      regression-test-first, STL Modeling Rules as HARD constraints, and
#      pre-staged per-model metadata + ports/gaskets sidecars.
#   2. The committed dogfood model under fixtures/dogfood/ runs through the REAL
#      release-gate engine to a GREEN gate (exit 0, no blocking stage=fail) and
#      the written release-gate.json is schema-valid (ajv). No mocks.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)"
SKILL_MD="$REPO_ROOT/skills/autospec-define/SKILL.md"
SCRIPTS_DIR="$REPO_ROOT/skills/autospec-fab/scripts"
DOGFOOD_DIR="$REPO_ROOT/skills/autospec-fab/fixtures/dogfood"
ENGINE="$SCRIPTS_DIR/stl-release-gate.py"

# --- Part A: fab lens prose -------------------------------------------------

@test "define fab lens: section header present" {
    grep -qi "fab.*decomposition\|fab lens\|3D-printable\|fab.yml" "$SKILL_MD"
}

@test "define fab lens: regression-test-first" {
    grep -qi "regression-test-first" "$SKILL_MD"
}

@test "define fab lens: STL Modeling Rules as hard constraints" {
    grep -qi "STL Modeling Rules" "$SKILL_MD"
    grep -qi "hard constraint" "$SKILL_MD"
}

@test "define fab lens: key modeling-rule tokens present" {
    # gasket wall, NPT access, vacuum-path/flow connectivity, watertight/single-body
    grep -qiE "5 ?mm" "$SKILL_MD"
    grep -qi "NPT" "$SKILL_MD"
    grep -qi "watertight" "$SKILL_MD"
    grep -qi "single-body" "$SKILL_MD"
}

@test "define fab lens: pre-stage metadata + ports/gaskets sidecars" {
    grep -qi "pre-stage" "$SKILL_MD"
    grep -qi "metadata" "$SKILL_MD"
    grep -qi "ports" "$SKILL_MD"
    grep -qi "gaskets" "$SKILL_MD"
}

# --- Part B: end-to-end dogfood -> green release-gate -----------------------

@test "dogfood fixtures committed" {
    [ -f "$DOGFOOD_DIR/dogfood.stl" ]
    [ -f "$DOGFOOD_DIR/metadata.json" ]
}

@test "dogfood model runs through the real gate to a GREEN release-gate.json" {
    command -v python3 || skip "python3 not found"
    out="$BATS_TEST_TMPDIR/release-gate.json"
    run python3 "$ENGINE" \
        --in "$DOGFOOD_DIR/dogfood.stl" \
        --model "$DOGFOOD_DIR/metadata.json" \
        --out "$out"
    echo "engine output: $output"
    # exit 0 == green gate (no blocking stage failed, schema valid)
    [ "$status" -eq 0 ]
    [ -f "$out" ]
    # No blocking stage may carry status=fail. (vision is advisory; it never
    # emits fail. Any fail anywhere means the gate is not green.)
    run python3 - "$out" <<'PY'
import json, sys
gate = json.load(open(sys.argv[1]))
bad = [s["stage"] for s in gate["stages"] if s["status"] == "fail"]
print("fail stages:", bad)
sys.exit(1 if bad else 0)
PY
    echo "fail-stage check: $output"
    [ "$status" -eq 0 ]
}

@test "dogfood release-gate.json is schema-valid (ajv)" {
    command -v ajv || skip "ajv not on PATH"
    command -v python3 || skip "python3 not found"
    out="$BATS_TEST_TMPDIR/release-gate.json"
    python3 "$ENGINE" \
        --in "$DOGFOOD_DIR/dogfood.stl" \
        --model "$DOGFOOD_DIR/metadata.json" \
        --out "$out"
    schema="$REPO_ROOT/schemas/autospec-fab-release-gate.schema.json"
    run ajv validate -s "$schema" --spec=draft2020 -d "$out"
    echo "ajv output: $output"
    [ "$status" -eq 0 ]
}
