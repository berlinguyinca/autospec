#!/usr/bin/env python3
"""
stage_vision.py — ADVISORY vision stage for autospec-fab.

Uniform stage CLI:
    stage_vision.py --in <contact-sheet.html | render-dir> --out <fragment.json>
                    [--rules <rules-file>] [--model <metadata.json>]

Behaviour (design spec §Release-gate stage 13 + decision 4 — "vision advises,
geometry gates"):
  1. Resolve the vision command from $AUTOSPEC_FAB_VISION_CMD, else PATH
     (shutil.which). Mocked via $TMP/bin in tests; real vision deferred to the
     pinned container. Absent -> status "skip", empty vision_findings.
  2. Resolve the contact sheet (the --in path itself, or contact-sheet.html
     inside the given render dir). Missing -> status "skip".
  3. JUDGE pass: subprocess the vision command with the contact sheet + the STL
     Modeling Rules; parse its JSON {"observations":[...]} into candidate
     advisory observations [{view?, observation, severity(info|warn), rule?}].
  4. ADVERSARIAL VERIFY: each raw observation is NOT trusted blindly. A light
     second pass re-queries the vision command in --verify mode with the
     candidate observation on stdin; only observations the command CONFIRMS are
     recorded. This filters hallucinated/low-confidence findings. (Documented
     and deliberately simple — the point is observations are filtered.)
  5. Emit the release-gate fragment. The confirmed observations go into the
     advisory **vision_findings** list, NEVER into the blocking findings[].
     status: "warn" if any confirmed observation is warn-severity, else "pass".
     The vision stage is ADVISORY: it NEVER emits "fail", even when it observes
     a hard-reject-flavoured defect (geometry gates block; vision only advises).

Exit codes: 0 — harness success (verdict lives in fragment "status"); 1 — usage
error. Stdlib only; the vision toolchain is external + PATH/env-resolved.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys

STAGE_NAME = "vision"
_VALID_SEVERITIES = ("info", "warn")


def resolve_vision_cmd() -> str | None:
    """
    Resolve the vision CLI: $AUTOSPEC_FAB_VISION_CMD first, else PATH lookup.

    Returns an absolute/usable command path, or None when no vision tool is
    available (tests mock this via $TMP/bin; real vision is deferred).
    """
    env_cmd = os.environ.get("AUTOSPEC_FAB_VISION_CMD")
    if env_cmd:
        if os.path.isfile(env_cmd) and os.access(env_cmd, os.X_OK):
            return env_cmd
        resolved = shutil.which(env_cmd)
        if resolved:
            return resolved
        return None
    return shutil.which("fab-vision")


def resolve_contact_sheet(in_path: str) -> str | None:
    """
    Resolve the contact-sheet artifact from --in.

    --in may be the contact-sheet.html file itself, or a render dir containing
    one. Returns the sheet path, or None if no contact sheet is found.
    """
    if os.path.isfile(in_path):
        return in_path
    if os.path.isdir(in_path):
        candidate = os.path.join(in_path, "contact-sheet.html")
        if os.path.isfile(candidate):
            return candidate
    return None


def _run_cmd(argv: list, stdin_text: str | None = None):
    """Run the vision command; return CompletedProcess or None on OSError."""
    try:
        return subprocess.run(
            argv,
            input=stdin_text,
            capture_output=True,
            text=True,
        )
    except OSError:
        return None


def _normalize_observation(raw: dict) -> dict | None:
    """
    Coerce a raw vision observation into the advisory shape, or None if invalid.

    Shape: {observation, severity(info|warn), view?, rule?}. Any unknown
    severity is clamped to "warn" so an advisory finding is never silently
    upgraded to a blocking severity.
    """
    if not isinstance(raw, dict):
        return None
    text = raw.get("observation")
    if not isinstance(text, str) or not text.strip():
        return None
    severity = raw.get("severity")
    if severity not in _VALID_SEVERITIES:
        severity = "warn"
    ob = {"observation": text.strip(), "severity": severity}
    if isinstance(raw.get("view"), str) and raw["view"]:
        ob["view"] = raw["view"]
    if isinstance(raw.get("rule"), str) and raw["rule"]:
        ob["rule"] = raw["rule"]
    return ob


def judge_contact_sheet(cmd: str, sheet: str, rules: str | None) -> list:
    """
    JUDGE pass: ask the vision command to review the sheet against the rules.

    Invokes `<cmd> <sheet> [<rules>]`; parses stdout JSON
    {"observations":[...]} into normalized candidate observations. Any parse or
    subprocess failure yields an empty candidate list (advisory: never raise).
    """
    argv = [cmd, sheet]
    if rules:
        argv.append(rules)
    proc = _run_cmd(argv)
    if proc is None or proc.returncode != 0:
        return []
    try:
        payload = json.loads(proc.stdout or "{}")
    except (ValueError, TypeError):
        return []
    raw_list = payload.get("observations") if isinstance(payload, dict) else None
    if not isinstance(raw_list, list):
        return []
    out = []
    for raw in raw_list:
        ob = _normalize_observation(raw)
        if ob is not None:
            out.append(ob)
    return out


def _confirm_observation(cmd: str, sheet: str, rules: str | None,
                         observation: dict) -> bool:
    """
    Adversarial second pass: re-query the vision command to confirm ONE
    observation before recording it.

    Invokes `<cmd> --verify <sheet> [<rules>]` with the candidate observation
    as JSON on stdin; the command answers {"confirmed": bool}. A missing or
    malformed verdict is treated as NOT confirmed (conservative: an unverified
    observation is dropped rather than trusted blindly).
    """
    argv = [cmd, "--verify", sheet]
    if rules:
        argv.append(rules)
    proc = _run_cmd(argv, stdin_text=json.dumps(observation))
    if proc is None or proc.returncode != 0:
        return False
    try:
        verdict = json.loads(proc.stdout or "{}")
    except (ValueError, TypeError):
        return False
    return isinstance(verdict, dict) and verdict.get("confirmed") is True


def verify_observations(cmd: str, sheet: str, rules: str | None,
                        candidates: list) -> list:
    """Keep only observations the adversarial verify pass confirms."""
    return [ob for ob in candidates
            if _confirm_observation(cmd, sheet, rules, ob)]


def _skip_fragment(detail: str) -> dict:
    """Advisory fragment for the vision-unavailable / no-sheet case."""
    return {
        "stage": STAGE_NAME,
        "status": "skip",
        "detail": detail,
        "findings": [],
        "vision_findings": [],
    }


def _advisory_status(findings: list) -> str:
    """
    Map confirmed advisory findings to a NON-BLOCKING status.

    "warn" if any confirmed observation is warn-severity, else "pass". This
    function deliberately CANNOT return "fail": the vision stage only advises.
    """
    if any(f.get("severity") == "warn" for f in findings):
        return "warn"
    return "pass"


def run_vision_stage(in_path: str, rules: str | None) -> dict:
    """
    Run the advisory vision stage and return a release-gate fragment.

    Fragment shape: {stage, status, detail, findings, vision_findings}. The
    confirmed advisory observations live in vision_findings; the blocking
    findings[] is ALWAYS empty (geometry gates block, vision only advises).
    """
    cmd = resolve_vision_cmd()
    if cmd is None:
        return _skip_fragment(
            "vision command not available; vision advisory deferred "
            "(real vision deferred to the pinned container)")

    sheet = resolve_contact_sheet(in_path)
    if sheet is None:
        return _skip_fragment(
            "no contact sheet at --in; vision advisory deferred")

    candidates = judge_contact_sheet(cmd, sheet, rules)
    confirmed = verify_observations(cmd, sheet, rules, candidates)

    status = _advisory_status(confirmed)
    detail = (
        f"vision advisory: {len(confirmed)} confirmed observation(s) "
        f"of {len(candidates)} candidate(s) (adversarially verified); "
        f"sheet: {sheet}"
    )
    return {
        "stage": STAGE_NAME,
        "status": status,
        "detail": detail,
        "findings": [],            # blocking list — vision NEVER populates it
        "vision_findings": confirmed,
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        description="autospec-fab advisory vision stage (non-blocking)"
    )
    parser.add_argument("--in", dest="in_path", required=True,
                        help="Contact-sheet HTML, or render dir containing one")
    parser.add_argument("--out", required=True,
                        help="Output release-gate fragment JSON path")
    parser.add_argument("--rules", dest="rules_path", default=None,
                        help="STL Modeling Rules file passed to the vision cmd")
    parser.add_argument("--model", dest="model_path", default=None,
                        help="Per-model metadata sidecar (optional, unused)")

    args = parser.parse_args(argv)

    fragment = run_vision_stage(args.in_path, args.rules_path)

    out_dir = os.path.dirname(os.path.abspath(args.out))
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)
    with open(args.out, "w") as f:
        json.dump(fragment, f, indent=2)
        f.write("\n")

    return 0


if __name__ == "__main__":
    sys.exit(main())
