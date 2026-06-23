#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# run_smoke.sh — real-solver integration smoke for autospec-fab (issue #1299,
# part of #1270). Drives ONE tiny fixture part through the four real-solver
# stages — fea + cfd + render + vision — end-to-end and asserts each returns a
# NON-`skip`, structurally-valid release-gate fragment.
#
# WHERE THIS RUNS: INSIDE the pinned #1300 container image, where the real
# binaries (ccx / simpleFoam / freecadcmd / fab-vision) are on PATH under the
# exact names the stage scripts resolve via shutil.which(). On a host WITHOUT
# those binaries every stage correctly `skip`s, so this script would (correctly)
# fail — that is the point: a skip-where-real-is-expected is a FAILURE here.
#
# DEFERRED: the full image build + this heavy run execute ONLY in the opt-in
# nightly/dispatch workflow (.github/workflows/fab-container.yml, child E /
# #1301 — deferred per operator). This child (#1299) authors the fixture +
# script; they are structurally gated host-portably by
# tests/test_fab_smoke_fixture.bats (no image build) and the validate.sh fab
# suite. The heavy run is NOT part of validate.sh.
#
# This is a WIRING / integration assertion, NOT a numerical-accuracy check: it
# proves the real toolchain integrates with the existing stage CLIs + result
# contracts, not that the solver answers are physically correct.
#
# Exit 0 only if all four stages ran (non-skip) with structurally-valid
# fragments; non-zero on any skip-where-real-expected or malformed fragment.
# ---------------------------------------------------------------------------
set -u

# --- locations -------------------------------------------------------------
SMOKE_DIR="$(cd "$(dirname "$0")" && pwd)"      # skills/autospec-fab/docker/smoke
SKILL_DIR="$(cd "$SMOKE_DIR/../.." && pwd)"     # skills/autospec-fab
SCRIPTS_DIR="$SKILL_DIR/scripts"
GATE="$SCRIPTS_DIR/stl-release-gate.py"

STL="$SMOKE_DIR/part.stl"
MODEL="$SMOKE_DIR/model/metadata.json"          # MODELDIR = $SMOKE_DIR/model

PYTHON="${PYTHON:-python3}"

# Per-run output dir (release-gate.json + renders/ land here). Mktemp keeps
# concurrent runs isolated and avoids polluting the read-only image layers.
WORK="$(mktemp -d "${TMPDIR:-/tmp}/fab-smoke.XXXXXX")"
OUT="$WORK/release-gate.json"

cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

fail() { printf 'SMOKE FAIL: %s\n' "$*" >&2; exit 1; }
note() { printf 'smoke: %s\n' "$*" >&2; }

# --- pre-flight ------------------------------------------------------------
command -v "$PYTHON" >/dev/null 2>&1 || fail "python3 not found on PATH"
[ -f "$GATE" ]  || fail "release-gate engine not found: $GATE"
[ -f "$STL" ]   || fail "fixture STL not found: $STL"
[ -f "$MODEL" ] || fail "fixture metadata not found: $MODEL"

# --- drive the full gate over the fixture part -----------------------------
# The gate threads MODELDIR sibling files (load.json -> fea, flow.json -> cfd)
# and the render->vision contact-sheet handoff automatically.
note "running release gate over $STL (model $MODEL)"
"$PYTHON" "$GATE" --in "$STL" --model "$MODEL" --out "$OUT" \
    --stages-dir "$SCRIPTS_DIR" >"$WORK/gate.log" 2>&1 \
    || { cat "$WORK/gate.log" >&2; fail "release gate engine crashed (harness error)"; }

[ -f "$OUT" ] || { cat "$WORK/gate.log" >&2; fail "gate produced no release-gate.json"; }

# --- per-stage NON-skip + structural assertions ----------------------------
# Each assertion below is run through python3 against the aggregated gate JSON.
# A helper returns the stage status (or empty if the stage record is missing)
# and we reject "skip" explicitly plus check the structured fragment shape.

# Assert a named stage exists and its status is NOT "skip" (and not empty/fail
# for the blocking stages we expect to PASS or at least RUN). We accept any
# non-skip status that proves the real solver executed; numerical pass/fail is
# out of scope (wiring assertion).
assert_stage_ran() {
    stage_name="$1"
    status="$("$PYTHON" - "$OUT" "$stage_name" <<'PY'
import json, sys
gate = json.load(open(sys.argv[1]))
name = sys.argv[2]
rec = next((s for s in gate.get("stages", []) if s.get("stage") == name), None)
print("" if rec is None else (rec.get("status") or ""))
PY
)" || fail "$stage_name: could not parse gate JSON"
    [ -n "$status" ] || fail "$stage_name: stage record missing from gate"
    [ "$status" != "skip" ] \
        || fail "$stage_name: status=skip — real solver did not run (expected non-skip inside the image)"
    note "$stage_name: status=$status (non-skip)"
}

# FEA: non-skip AND structured fea_results.safety_factor present + numeric.
assert_fea() {
    assert_stage_ran fea
    "$PYTHON" - "$OUT" <<'PY' || fail "fea: malformed fea_results (no numeric safety_factor)"
import json, sys
gate = json.load(open(sys.argv[1]))
res = gate.get("fea_results")
assert isinstance(res, dict), "fea_results missing or not an object"
sf = res.get("safety_factor")
assert isinstance(sf, (int, float)), "safety_factor missing or non-numeric: %r" % (sf,)
PY
    note "fea: fea_results.safety_factor present + numeric"
}

# CFD: non-skip AND parsed cfd_results.{pressure_drop_pa,min_velocity_m_s}.
assert_cfd() {
    assert_stage_ran cfd
    "$PYTHON" - "$OUT" <<'PY' || fail "cfd: malformed cfd_results (PRESSURE_DROP/MIN_VELOCITY not parsed)"
import json, sys
gate = json.load(open(sys.argv[1]))
res = gate.get("cfd_results")
assert isinstance(res, dict), "cfd_results missing or not an object"
dp = res.get("pressure_drop_pa")
vel = res.get("min_velocity_m_s")
assert isinstance(dp, (int, float)), "pressure_drop_pa missing/non-numeric: %r" % (dp,)
assert isinstance(vel, (int, float)), "min_velocity_m_s missing/non-numeric: %r" % (vel,)
PY
    note "cfd: cfd_results PRESSURE_DROP + MIN_VELOCITY parsed"
}

# RENDER: non-skip AND a real contact-sheet.html + at least one PNG on disk.
# render_dir = <out-dir>/renders ; slug = STL stem (release_gate_stages.py).
assert_render() {
    assert_stage_ran render
    slug="$(basename "$STL")"; slug="${slug%.stl}"
    [ -n "$slug" ] || slug="model"
    render_subdir="$WORK/renders/$slug"
    sheet="$render_subdir/contact-sheet.html"
    [ -f "$sheet" ] || fail "render: contact-sheet.html not produced at $sheet"
    # at least one rendered PNG view present
    png_count=0
    for png in "$render_subdir"/*.png; do
        [ -f "$png" ] && png_count=$((png_count + 1))
    done
    [ "$png_count" -gt 0 ] || fail "render: no PNG views produced under $render_subdir"
    note "render: contact-sheet.html + $png_count PNG view(s) produced"
}

# VISION: non-skip AND a structurally-valid (possibly empty) vision_findings list.
assert_vision() {
    assert_stage_ran vision
    "$PYTHON" - "$OUT" <<'PY' || fail "vision: vision_findings is not a structurally-valid list"
import json, sys
gate = json.load(open(sys.argv[1]))
vf = gate.get("vision_findings")
assert isinstance(vf, list), "vision_findings missing or not a list: %r" % (type(vf),)
for f in vf:
    assert isinstance(f, dict), "vision finding is not an object: %r" % (f,)
PY
    note "vision: vision_findings structurally valid (count from list)"
}

assert_fea
assert_cfd
assert_render
assert_vision

note "ALL STAGES RAN NON-SKIP WITH STRUCTURALLY-VALID FRAGMENTS"
note "release-gate.json: $OUT (removed on exit)"
exit 0
