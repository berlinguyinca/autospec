#!/usr/bin/env bash
# qa-finding-filter.sh — post-cluster finding filter for autospec-qa
# (issue #647).
#
# Enforces verify-first discipline AFTER the QA orchestrator merges cluster
# output into .autospec/qa-verdict.json. Partitions findings into:
#   verified[]   — verified_at == current HEAD; fresh probe → keep
#   stale[]      — verified_at present but != HEAD; re-probe per category
#   unverified[] — verified_at missing entirely
#
# Strict mode (default in CI; controlled via AUTOSPEC_QA_STRICT=1) fails the
# run when any finding is unverified. Non-strict mode moves unverified
# findings into unverified_warnings[] and exits 0 so the operator can triage.
#
# Stale findings are re-probed via qa-verify-finding.sh against the recorded
# verify_category + verify_args (if any). Probe exit 1 ("resolved") drops the
# finding into verified_dropped[]; probe exit 0 ("still reproduces") refreshes
# verified_at to current HEAD and keeps the finding.
#
# Output is rewritten atomically via a sibling tmp file + mv.
#
# Exit codes:
#   0  — filter completed (verdict file may have been rewritten)
#   1  — strict mode violation: at least one unverified finding present
#   2  — usage / bad arguments

set -u

usage() {
    cat <<'EOF'
Usage: qa-finding-filter.sh --verdict <path>

Reads --verdict path, partitions findings, atomically rewrites the file.

Env:
  AUTOSPEC_QA_STRICT       1 (default) → fail on unverified findings;
                           0           → move them to unverified_warnings[]
  AUTOSPEC_SCRIPTS_DIR     directory containing qa-verify-finding.sh
                           (defaults to the directory of this script)

Exit 0: filter ran (verdict possibly rewritten).
Exit 1: strict mode found unverified findings; verdict left unchanged.
Exit 2: bad usage.
EOF
}

VERDICT=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --verdict) VERDICT="$2"; shift 2 ;;
        -h|--help) usage; exit 2 ;;
        *) echo "qa-finding-filter: unknown arg: $1" >&2; usage; exit 2 ;;
    esac
done

if [ -z "$VERDICT" ]; then
    echo "qa-finding-filter: --verdict required" >&2
    usage
    exit 2
fi

if [ ! -f "$VERDICT" ]; then
    echo "qa-finding-filter: verdict file not found: $VERDICT" >&2
    exit 2
fi

STRICT="${AUTOSPEC_QA_STRICT:-1}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VERIFY_SH="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}/qa-verify-finding.sh"
if [ ! -x "$VERIFY_SH" ] && [ -x "$SCRIPT_DIR/qa-verify-finding.sh" ]; then
    VERIFY_SH="$SCRIPT_DIR/qa-verify-finding.sh"
fi

REPO_TOP="$(git rev-parse --show-toplevel 2>/dev/null || echo "")"
if [ -z "$REPO_TOP" ]; then
    echo "qa-finding-filter: not inside a git repo (cannot compute HEAD)" >&2
    exit 2
fi
# Resolve verdict to absolute path before chdir so the rewrite finds it.
case "$VERDICT" in
    /*) ;;
    *)  VERDICT="$(cd "$(dirname "$VERDICT")" && pwd)/$(basename "$VERDICT")" ;;
esac
TMP_OUT="${VERDICT}.tmp"
cd "$REPO_TOP"
HEAD_SHA="$(git rev-parse HEAD)"

if ! command -v python3 >/dev/null 2>&1; then
    echo "qa-finding-filter: python3 required for JSON manipulation" >&2
    exit 2
fi

# Hand off to python3 for JSON-safe partitioning + re-probe orchestration.
VERDICT="$VERDICT" \
TMP_OUT="$TMP_OUT" \
HEAD_SHA="$HEAD_SHA" \
STRICT="$STRICT" \
VERIFY_SH="$VERIFY_SH" \
python3 - <<'PY'
import json
import os
import subprocess
import sys

verdict_path = os.environ["VERDICT"]
tmp_out      = os.environ["TMP_OUT"]
head_sha     = os.environ["HEAD_SHA"]
strict       = os.environ.get("STRICT", "1") == "1"
verify_sh    = os.environ["VERIFY_SH"]

with open(verdict_path, "r") as f:
    try:
        verdict = json.load(f)
    except json.JSONDecodeError as e:
        print(f"qa-finding-filter: verdict is not valid JSON: {e}", file=sys.stderr)
        sys.exit(2)

findings = verdict.get("findings", []) or []

verified            = []
verified_dropped    = list(verdict.get("verified_dropped", []) or [])
unverified_warnings = list(verdict.get("unverified_warnings", []) or [])
unverified_strict   = []

def reprobe(finding):
    """Run qa-verify-finding.sh against finding's recorded category + args.
    Returns probe exit code (0 keep, 1 drop, 2 bad args)."""
    cat = finding.get("verify_category") or finding.get("category")
    if not cat:
        return 2
    args = ["bash", verify_sh, "--category", cat]
    va = finding.get("verify_args") or {}
    for k, v in va.items():
        args.extend([f"--{k}", str(v)])
    try:
        rc = subprocess.run(args, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL).returncode
    except Exception:
        rc = 2
    return rc

for f in findings:
    va = f.get("verified_at")
    if not va:
        if strict:
            unverified_strict.append(f)
        else:
            unverified_warnings.append(f)
        continue
    if va == head_sha:
        verified.append(f)
        continue
    # Stale: re-probe.
    rc = reprobe(f)
    if rc == 1:
        # Resolved — drop.
        dropped = dict(f)
        dropped["dropped_reason"] = "stale_reprobe_resolved"
        verified_dropped.append(dropped)
    elif rc == 0:
        # Still reproduces — refresh verified_at.
        refreshed = dict(f)
        refreshed["verified_at"] = head_sha
        refreshed["verified_by"] = "qa-finding-filter.sh"
        verified.append(refreshed)
    else:
        # Probe couldn't run — treat as unverified per strict policy.
        if strict:
            unverified_strict.append(f)
        else:
            warn = dict(f)
            warn["unverified_reason"] = "reprobe_indeterminate"
            unverified_warnings.append(warn)

if strict and unverified_strict:
    print("qa-finding-filter: strict mode violation — "
          "code_health:verify_first_violation", file=sys.stderr)
    for f in unverified_strict:
        summary = f.get("summary", "<no summary>")
        cat     = f.get("category", "<no category>")
        print(f"  unverified[{cat}]: {summary}", file=sys.stderr)
    # Leave verdict file untouched; operator must re-run cluster with probe.
    sys.exit(1)

verdict["findings"]            = verified
verdict["verified_dropped"]    = verified_dropped
verdict["unverified_warnings"] = unverified_warnings

with open(tmp_out, "w") as out:
    json.dump(verdict, out, indent=2)
    out.write("\n")
os.replace(tmp_out, verdict_path)
PY

rc=$?
exit "$rc"
