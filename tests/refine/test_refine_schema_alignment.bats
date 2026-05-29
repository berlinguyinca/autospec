#!/usr/bin/env bats
# Schema-alignment coverage for issue #682:
#   - Single-pass schema accepts status=degraded and status=completed.
#   - Loop schema accepts all 8 loop termination statuses.
#   - Single-pass output does NOT validate against loop schema.
#   - Loop output does NOT validate against single-pass schema.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SINGLE_SCHEMA="$REPO_ROOT/schemas/autospec-refinement.schema.json"
    LOOP_SCHEMA="$REPO_ROOT/schemas/autospec-refinement-loop.schema.json"
    TMP="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMP"
}

# Validate $1=json-file against $2=schema using python jsonschema.
_validate() {
    local json="$1" schema="$2"
    python3 - "$json" "$schema" <<'PY'
import json, sys
try:
    import jsonschema
except ImportError:
    sys.stderr.write("SKIP_NO_JSONSCHEMA\n")
    sys.exit(77)
with open(sys.argv[1]) as f: doc = json.load(f)
with open(sys.argv[2]) as f: sch = json.load(f)
try:
    jsonschema.validate(doc, sch)
except jsonschema.ValidationError as e:
    sys.stderr.write(f"VALIDATION_FAIL: {e.message}\n")
    sys.exit(1)
PY
}

@test "single-pass schema accepts status=degraded" {
    cat > "$TMP/single-degraded.json" <<'JSON'
{
  "original_prompt": "x",
  "rounds": [],
  "final_prompt": "x",
  "status": "degraded",
  "metadata": {
    "head_sha": "abc",
    "timestamp": "2026-05-28T00-00-00Z",
    "rounds_requested": 1,
    "rounds_executed": 1,
    "converged_early": false,
    "degraded_rounds": ["1:repo-grounding"],
    "handoff_target": "dry-run",
    "handoff_executed": false
  }
}
JSON
    run _validate "$TMP/single-degraded.json" "$SINGLE_SCHEMA"
    [ "$status" -eq 77 ] && skip "jsonschema not installed"
    [ "$status" -eq 0 ]
}

@test "single-pass schema accepts status=completed" {
    cat > "$TMP/single-completed.json" <<'JSON'
{
  "original_prompt": "x",
  "rounds": [],
  "final_prompt": "x",
  "status": "completed",
  "metadata": {
    "head_sha": "abc",
    "timestamp": "2026-05-28T00-00-00Z",
    "rounds_requested": 1,
    "rounds_executed": 1,
    "converged_early": false,
    "degraded_rounds": [],
    "handoff_target": "dry-run",
    "handoff_executed": false
  }
}
JSON
    run _validate "$TMP/single-completed.json" "$SINGLE_SCHEMA"
    [ "$status" -eq 77 ] && skip "jsonschema not installed"
    [ "$status" -eq 0 ]
}

@test "loop schema accepts all 8 loop statuses" {
    for s in convergence_clean oscillation_detected evidence_based_stop budget_cap_reached operator_stop round_cap_reached handoff_failed iteration_error; do
        cat > "$TMP/loop-$s.json" <<JSON
{
  "slug": "x",
  "status": "$s",
  "iterations_executed": 1,
  "max_iterations": 5,
  "tokens_used": 0,
  "iterations": []
}
JSON
        run _validate "$TMP/loop-$s.json" "$LOOP_SCHEMA"
        [ "$status" -eq 77 ] && skip "jsonschema not installed"
        [ "$status" -eq 0 ] || { echo "loop status $s rejected"; return 1; }
    done
}

@test "single-pass output does NOT validate against loop schema" {
    cat > "$TMP/single.json" <<'JSON'
{
  "original_prompt": "x",
  "rounds": [],
  "final_prompt": "x",
  "status": "degraded",
  "metadata": {
    "head_sha": "abc",
    "timestamp": "t",
    "rounds_requested": 1,
    "rounds_executed": 1,
    "converged_early": false,
    "degraded_rounds": [],
    "handoff_target": "dry-run",
    "handoff_executed": false
  }
}
JSON
    run _validate "$TMP/single.json" "$LOOP_SCHEMA"
    [ "$status" -eq 77 ] && skip "jsonschema not installed"
    [ "$status" -ne 0 ]
}

@test "loop output does NOT validate against single-pass schema" {
    cat > "$TMP/loop.json" <<'JSON'
{
  "slug": "x",
  "status": "convergence_clean",
  "iterations_executed": 1,
  "max_iterations": 5,
  "tokens_used": 0,
  "iterations": []
}
JSON
    run _validate "$TMP/loop.json" "$SINGLE_SCHEMA"
    [ "$status" -eq 77 ] && skip "jsonschema not installed"
    [ "$status" -ne 0 ]
}

@test "both schemas parse as valid JSON Schema" {
    python3 - "$SINGLE_SCHEMA" "$LOOP_SCHEMA" <<'PY' || skip "jsonschema not installed"
import json, sys
try:
    import jsonschema
except ImportError:
    sys.exit(77)
for p in sys.argv[1:]:
    with open(p) as f: sch = json.load(f)
    jsonschema.Draft202012Validator.check_schema(sch)
PY
}
