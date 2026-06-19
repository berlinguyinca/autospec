#!/usr/bin/env python3
"""stl-release-gate.py — the autospec-fab RELEASE-GATE ENGINE.

Sequences the 12 fab stages in composition order, runs each with the uniform
stage CLI (`--in <stl> --model <metadata> --out <fragment>`), aggregates their
per-stage fragments into a schema-valid `.autospec/fab/release-gate.json`, and
decides the overall verdict.

Verdict / exit code:
  - The gate FAILS (exit non-zero) if ANY blocking stage fragment has
    status "fail".
  - The vision stage is ADVISORY (blocking=False): its warn/fail never blocks;
    its observations are surfaced in release-gate.json["vision_findings"].
  - "skip" never fails. A green gate (all blocking stages pass/skip) exits 0.

Each stage script exits non-zero only on a HARNESS error (the gate verdict is
expressed in the fragment "status", not the stage exit code). A harness error
in a blocking stage is treated as a blocking failure.

The written gate is schema-validated with the `ajv` CLI when present; if ajv is
absent, validation is skipped with a warning (the engine never crashes on a
missing validator).

CLI:
    stl-release-gate.py --in <stl> --model <metadata.json>
                        --out <release-gate.json> [--stages-dir <dir>]

Stage scripts resolve from (in priority order): --stages-dir,
$AUTOSPEC_FAB_STAGES_DIR, then the engine's own scripts directory.

Stdlib only; runs on bare python3 in the fab gate.
"""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile

from release_gate_stages import (
    STAGE_ORDER, contact_sheet_for, extra_args_for, render_dir_for,
    resolve_stage_script)

# Keys the schema permits on a stage record; the engine strips everything else
# off each fragment before aggregating (fragments may carry extras like
# vision_findings or harness-internal keys).
_STAGE_KEYS = ("stage", "status", "detail", "findings")


def _geometry_hash(stl_path):
    """Return the sha256 hex digest of the STL file (or "" if unreadable)."""
    try:
        h = hashlib.sha256()
        with open(stl_path, "rb") as f:
            for chunk in iter(lambda: f.read(65536), b""):
                h.update(chunk)
        return h.hexdigest()
    except OSError:
        return ""


def _model_name(model_path):
    """Best-effort model name from the metadata sidecar (or "")."""
    try:
        with open(model_path, encoding="utf-8") as f:
            data = json.load(f)
        name = data.get("name")
        return name if isinstance(name, str) else ""
    except (OSError, ValueError):
        return ""


def _stage_argv(script, stage, stl_path, model_path, out_path, ctx):
    """Build the per-stage argv, threading sibling-file + handoff inputs.

    Every stage gets `--model` + `--out`. The default input is `--in <stl>`,
    UNLESS the resolver/handoff already supplies its own `--in` (docs takes the
    model DIR; vision takes the render's contact sheet). render gets a
    deterministic `--render-dir` (ctx["render_dir"]). `ctx["in_override"]` maps
    a stage name -> an `--in` value (used to hand vision the render sheet).
    """
    name = stage["name"]
    extra = extra_args_for(name, stl_path, model_path, ctx.get("render_dir"))
    argv = [sys.executable, script, "--model", model_path, "--out", out_path]
    override = ctx.get("in_override", {}).get(name)
    if override is not None:
        argv += ["--in", override]
    elif "--in" not in extra:
        argv += ["--in", stl_path]
    return argv + extra


def _run_stage(stage, stl_path, model_path, stages_dir, ctx):
    """Run one stage; return (fragment_dict, harness_ok).

    fragment_dict is the raw JSON the stage wrote (or a synthetic error
    fragment if the stage produced none). harness_ok is False when the stage
    crashed or emitted no usable fragment. `ctx` carries the per-run handoff
    state (render dir + per-stage --in overrides).
    """
    script = resolve_stage_script(stage["script"], stages_dir)
    if script is None:
        return ({"stage": stage["name"], "status": "fail",
                 "detail": "stage script not found: %s" % stage["script"],
                 "findings": []}, False)

    tmp_out = tempfile.NamedTemporaryFile(
        prefix="fab-frag-", suffix=".json", delete=False)
    tmp_out.close()
    try:
        argv = _stage_argv(
            script, stage, stl_path, model_path, tmp_out.name, ctx)
        proc = subprocess.run(argv, capture_output=True, text=True)
        fragment = _load_fragment(tmp_out.name, stage["name"])
        harness_ok = proc.returncode == 0 and fragment is not None
        if fragment is None:
            detail = "no fragment emitted (rc=%d): %s" % (
                proc.returncode, (proc.stderr or "").strip()[:200])
            fragment = {"stage": stage["name"], "status": "fail",
                        "detail": detail, "findings": []}
        return (fragment, harness_ok)
    finally:
        try:
            os.unlink(tmp_out.name)
        except OSError:
            pass


def _load_fragment(path, stage_name):
    """Load a stage fragment JSON; return None if missing/invalid."""
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError):
        return None
    if not isinstance(data, dict):
        return None
    data.setdefault("stage", stage_name)
    return data


def _clean_stage_record(fragment, stage_name):
    """Project a fragment down to the schema-permitted stage-record keys."""
    record = {"stage": fragment.get("stage", stage_name),
              "status": fragment.get("status", "fail")}
    if "detail" in fragment:
        record["detail"] = fragment["detail"]
    findings = fragment.get("findings")
    if isinstance(findings, list):
        record["findings"] = findings
    else:
        record["findings"] = []
    return record


def _vision_in_override(render_dir, stl_path):
    """The vision stage's --in: the render's produced contact sheet.

    Always the deterministic sheet path under the render dir (NOT the STL). If
    render emitted no sheet (renderer absent → skip), this path won't exist and
    vision's resolve_contact_sheet returns None → vision skips gracefully.
    """
    return contact_sheet_for(render_dir, stl_path)


def run_gate(stl_path, model_path, stages_dir, out_path):
    """Run every stage in order; return (gate_dict, blocking_failed: bool).

    Threads the render→vision handoff: render gets a deterministic
    --render-dir; vision's --in is pointed at the contact sheet render writes
    there (resolved AFTER render ran, so an absent sheet makes vision skip).
    """
    stages = []
    vision_findings = []
    extras = {}  # top-level structured results keyed by gate field name
    blocking_failed = False
    render_dir = render_dir_for(out_path)
    ctx = {"render_dir": render_dir, "in_override": {}}

    for stage in STAGE_ORDER:
        if stage["name"] == "vision":
            ctx["in_override"]["vision"] = _vision_in_override(
                render_dir, stl_path)
        fragment, harness_ok = _run_stage(
            stage, stl_path, model_path, stages_dir, ctx)
        record = _clean_stage_record(fragment, stage["name"])
        stages.append(record)
        _collect_stage_extras(stage["name"], fragment, vision_findings, extras)

        if stage["blocking"]:
            if record["status"] == "fail" or not harness_ok:
                blocking_failed = True

    gate = {
        "schema_version": 1,
        "name": _model_name(model_path),
        "geometry_hash": _geometry_hash(stl_path),
        "stages": stages,
        "vision_findings": vision_findings,
    }
    gate.update(extras)
    return gate, blocking_failed


# Stage name -> the top-level gate field its structured results object feeds.
_STRUCTURED_RESULT_FIELDS = {"fea": "fea_results", "cfd": "cfd_results"}


def _collect_stage_extras(name, fragment, vision_findings, extras):
    """Mirror per-stage extras into the gate top level.

    vision -> advisory vision_findings (appended); fea/cfd -> structured
    fea_results/cfd_results dicts (placed at gate top-level, parallel to
    vision_findings). Absent/ill-typed extras are skipped (no fabrication).
    """
    if name == "vision":
        vf = fragment.get("vision_findings")
        if isinstance(vf, list):
            vision_findings.extend(vf)
        return
    field = _STRUCTURED_RESULT_FIELDS.get(name)
    if field is None:
        return
    value = fragment.get(field)
    if isinstance(value, dict):
        extras[field] = value


def _ajv_validate(schema_path, out_path):
    """Validate the written gate with ajv; return True on valid/skip."""
    if shutil.which("ajv") is None:
        print("WARN: ajv not on PATH; skipping schema validation",
              file=sys.stderr)
        return True
    proc = subprocess.run(
        ["ajv", "validate", "-s", schema_path, "--spec=draft2020",
         "-d", out_path],
        capture_output=True, text=True)
    if proc.returncode != 0:
        print("ERROR: release-gate.json failed schema validation:",
              file=sys.stderr)
        print((proc.stdout or "") + (proc.stderr or ""), file=sys.stderr)
        return False
    return True


def _schema_path():
    """Locate the release-gate schema relative to the engine's repo root."""
    here = os.path.dirname(os.path.abspath(__file__))
    repo_root = os.path.abspath(os.path.join(here, "..", "..", ".."))
    return os.path.join(
        repo_root, "schemas", "autospec-fab-release-gate.schema.json")


def _summary(gate, blocking_failed):
    """One-line-per-stage human summary for stderr."""
    lines = ["release-gate: %s" % (gate.get("name") or "<unnamed>")]
    for s in gate["stages"]:
        lines.append("  %-14s %s" % (s["stage"], s["status"]))
    if gate["vision_findings"]:
        lines.append("  vision advisory findings: %d"
                     % len(gate["vision_findings"]))
    lines.append("verdict: %s"
                 % ("FAIL (blocking stage failed)" if blocking_failed
                    else "PASS (green gate)"))
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="autospec-fab release-gate engine: sequence stages and "
                    "emit a schema-valid release-gate.json")
    parser.add_argument("--in", dest="in_path", required=True,
                        help="STL file (or model dir) to gate")
    parser.add_argument("--model", dest="model_path", required=True,
                        help="per-model metadata sidecar JSON")
    parser.add_argument("--out", required=True,
                        help="output release-gate.json path")
    parser.add_argument("--stages-dir", dest="stages_dir", default=None,
                        help="override directory holding stage scripts")
    args = parser.parse_args(argv)

    gate, blocking_failed = run_gate(
        args.in_path, args.model_path, args.stages_dir, args.out)

    out_dir = os.path.dirname(os.path.abspath(args.out))
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(gate, f, indent=2)
        f.write("\n")

    schema_valid = _ajv_validate(_schema_path(), args.out)
    print(_summary(gate, blocking_failed), file=sys.stderr)

    if not schema_valid:
        # A gate that fails its own schema is an engine bug; fail loudly.
        return 2
    return 1 if blocking_failed else 0


if __name__ == "__main__":
    sys.exit(main())
