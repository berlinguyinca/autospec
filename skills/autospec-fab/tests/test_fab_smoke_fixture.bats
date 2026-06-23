#!/usr/bin/env bats
# test_fab_smoke_fixture.bats — HOST-PORTABLE structural gate for the
# real-solver smoke (issue #1299, part of #1270). Runs WITHOUT building the
# multi-GB #1300 image: it asserts the smoke script + MODELDIR fixture are
# present, well-formed, and wired to the stages that should run. The actual
# heavy run (real ccx/simpleFoam/freecadcmd/fab-vision) executes only inside
# the pinned image via the deferred nightly/dispatch fab-container.yml.
#
# Auto-discovered by validate.sh's check_autospec_fab_contract, which runs
# every skills/autospec-fab/tests/*.bats file.

# tests/ -> skill/ -> skills/ -> repo root
REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)"
SMOKE_DIR="$REPO_ROOT/skills/autospec-fab/docker/smoke"
MODEL_DIR="$SMOKE_DIR/model"
RUN_SMOKE="$SMOKE_DIR/run_smoke.sh"

@test "run_smoke.sh exists" {
    [ -f "$RUN_SMOKE" ]
}

@test "run_smoke.sh is bash -n clean" {
    run bash -n "$RUN_SMOKE"
    [ "$status" -eq 0 ]
}

@test "run_smoke.sh uses set -u (bash-3.2-safe discipline)" {
    grep -Eq '^set -u' "$RUN_SMOKE"
}

@test "run_smoke.sh does not use [ -f <(...) ] (bash 3.2 process-sub trap)" {
    # macOS bash 3.2: [ -f <(...) ] is always false. Forbid the pattern.
    run grep -nE '\[ +-f +<\(' "$RUN_SMOKE"
    [ "$status" -ne 0 ]
}

@test "run_smoke.sh rejects skip-where-real-expected for every stage" {
    # The non-skip assertion is the heart of the smoke: it must explicitly
    # reject status=skip.
    grep -q '!= "skip"' "$RUN_SMOKE"
}

@test "run_smoke.sh asserts all four real-solver stages" {
    grep -q 'assert_fea' "$RUN_SMOKE"
    grep -q 'assert_cfd' "$RUN_SMOKE"
    grep -q 'assert_render' "$RUN_SMOKE"
    grep -q 'assert_vision' "$RUN_SMOKE"
}

@test "run_smoke.sh asserts FEA structured safety_factor" {
    grep -q 'safety_factor' "$RUN_SMOKE"
}

@test "run_smoke.sh asserts CFD PRESSURE_DROP + MIN_VELOCITY" {
    grep -q 'pressure_drop_pa' "$RUN_SMOKE"
    grep -q 'min_velocity_m_s' "$RUN_SMOKE"
}

@test "run_smoke.sh asserts render PNG + contact-sheet" {
    grep -q 'contact-sheet.html' "$RUN_SMOKE"
    grep -q '\.png' "$RUN_SMOKE"
}

@test "run_smoke.sh asserts structurally-valid vision_findings" {
    grep -q 'vision_findings' "$RUN_SMOKE"
}

@test "run_smoke.sh header documents deferred-to-image execution" {
    grep -qi 'INSIDE the pinned' "$RUN_SMOKE"
    grep -qi 'fab-container.yml' "$RUN_SMOKE"
}

@test "fixture part.stl is present and non-empty" {
    [ -s "$SMOKE_DIR/part.stl" ]
}

@test "MODELDIR sidecars are present" {
    [ -f "$MODEL_DIR/metadata.json" ]
    [ -f "$MODEL_DIR/load.json" ]
    [ -f "$MODEL_DIR/flow.json" ]
}

@test "MODELDIR sidecars are valid JSON" {
    command -v python3 >/dev/null 2>&1 || skip "python3 not found"
    for f in metadata.json load.json flow.json; do
        run python3 -m json.tool "$MODEL_DIR/$f"
        [ "$status" -eq 0 ]
    done
}

@test "metadata.json marks the part load_critical + flow_critical" {
    command -v python3 >/dev/null 2>&1 || skip "python3 not found"
    # load_critical=true is the gate flag that lets FEA actually RUN (paired
    # with the load.json sibling); flow_critical=true does the same for CFD.
    run python3 -c "import json,sys; m=json.load(open(sys.argv[1])); sys.exit(0 if m.get('load_critical') is True and m.get('flow_critical') is True else 1)" "$MODEL_DIR/metadata.json"
    [ "$status" -eq 0 ]
}

@test "load.json carries the FEA load spec (force_n, safety_factor_min)" {
    command -v python3 >/dev/null 2>&1 || skip "python3 not found"
    run python3 -c "import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if 'force_n' in d and 'safety_factor_min' in d else 1)" "$MODEL_DIR/load.json"
    [ "$status" -eq 0 ]
}

@test "flow.json carries the CFD flow spec (max_pressure_drop_pa, min_velocity_m_s)" {
    command -v python3 >/dev/null 2>&1 || skip "python3 not found"
    run python3 -c "import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if 'max_pressure_drop_pa' in d and 'min_velocity_m_s' in d else 1)" "$MODEL_DIR/flow.json"
    [ "$status" -eq 0 ]
}

@test "sibling files map to the stages that should run (_SIBLING_INPUTS convention)" {
    command -v python3 >/dev/null 2>&1 || skip "python3 not found"
    # Cross-check the fixture against the engine's own sibling-file table: the
    # fixture's load.json/flow.json must be exactly the filenames the engine
    # threads into fea/cfd. Guards against a fixture/engine convention drift.
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-fab/scripts"
    run env PYTHONPATH="$SCRIPTS_DIR" MODEL_DIR="$MODEL_DIR" python3 - <<'PY'
import os
from release_gate_stages import _SIBLING_INPUTS
model_dir = os.environ["MODEL_DIR"]
# fea -> load.json, cfd -> flow.json per the engine table.
fea_flag, fea_file = _SIBLING_INPUTS["fea"]
cfd_flag, cfd_file = _SIBLING_INPUTS["cfd"]
assert fea_file == "load.json", fea_file
assert cfd_file == "flow.json", cfd_file
assert os.path.isfile(os.path.join(model_dir, fea_file)), "missing fea sibling"
assert os.path.isfile(os.path.join(model_dir, cfd_file)), "missing cfd sibling"
PY
    [ "$status" -eq 0 ]
}
