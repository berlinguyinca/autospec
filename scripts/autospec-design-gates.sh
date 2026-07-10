#!/usr/bin/env bash
# scripts/autospec-design-gates.sh — execute baseline-pack design gates (rules.yaml check: auto).
#
# Consumes a repo-local .autospec/design-gates.yml that maps machine-checkable
# rule ids from a baseline pack's rules.yaml to concrete local commands, runs
# the mapped gates, and writes an evidence report for QA / the premerge gate.
# Opt-in: repos without the config skip cleanly (exit 0, status "skipped").
set -eu

usage() {
  cat <<'EOF'
Usage:
  autospec-design-gates.sh [--repo-root <dir>] [--config <path>]
                           [--changed-files <file>] [--strict]

Config (.autospec/design-gates.yml):
  rules_file: <path to the baseline pack rules.yaml>          # required
  pack_file:  <path to the pack .pack.json>                   # optional
  ui_paths:                                                   # optional
    - "src/**"
  gates:
    <rule-id>:
      command: "<shell command; exit 0 = pass>"
      blocking: true|false        # optional; default: severity == blocker

Behavior:
  - No config file            -> status "skipped", exit 0.
  - --changed-files given and no file matches ui_paths -> "skipped_not_ui", exit 0.
  - check: auto rules with a mapped command are executed; unmapped ones are
    recorded as advisory ("unmapped") unless --strict, which makes unmapped
    blocker-severity rules fail the gate.
  - check: vlm / review rules and pack qualityGates are emitted as a checklist
    for the QA critic; they are never executed here.

Writes:
  .autospec/reports/design-gates.json
  .autospec/reports/design-gates.md

Final stdout line (parse this, not the exit code, in pipelines):
  autospec-design-gates: PASS|FAIL|SKIPPED (<run> run, <failed> failed, <unmapped> unmapped)

Exit codes: 0 pass or skipped; 1 blocking gate failed; 2 invocation/config error.
EOF
}

die() { printf 'autospec-design-gates: %s\n' "$*" >&2; exit 2; }

REPO_ROOT="$(pwd)"
CONFIG_PATH=""
CHANGED_FILES=""
STRICT=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
    --config) [ "$#" -ge 2 ] || die "--config requires a value"; CONFIG_PATH="$2"; shift 2 ;;
    --changed-files) [ "$#" -ge 2 ] || die "--changed-files requires a value"; CHANGED_FILES="$2"; shift 2 ;;
    --strict) STRICT=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown arg: $1" ;;
  esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"
if [ -z "$CONFIG_PATH" ]; then
  CONFIG_PATH="$REPO_ROOT/.autospec/design-gates.yml"
fi
if [ "${CONFIG_PATH#/}" = "$CONFIG_PATH" ]; then
  CONFIG_PATH="$REPO_ROOT/$CONFIG_PATH"
fi
if [ -n "$CHANGED_FILES" ] && [ ! -f "$CHANGED_FILES" ]; then
  die "--changed-files does not exist: $CHANGED_FILES"
fi

export AUTOSPEC_DG_REPO_ROOT="$REPO_ROOT"
export AUTOSPEC_DG_CONFIG="$CONFIG_PATH"
export AUTOSPEC_DG_CHANGED="${CHANGED_FILES:-}"
export AUTOSPEC_DG_STRICT="$STRICT"

python3 - <<'PY'
import datetime, json, os, re, subprocess, sys

repo_root = os.environ["AUTOSPEC_DG_REPO_ROOT"]
config_path = os.environ["AUTOSPEC_DG_CONFIG"]
changed_path = os.environ.get("AUTOSPEC_DG_CHANGED", "")
strict = os.environ.get("AUTOSPEC_DG_STRICT", "0") == "1"

reports_dir = os.path.join(repo_root, ".autospec", "reports")
json_out = os.path.join(reports_dir, "design-gates.json")
md_out = os.path.join(reports_dir, "design-gates.md")


def now_iso():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def finish(report, exit_code):
    os.makedirs(reports_dir, exist_ok=True)
    with open(json_out, "w") as fh:
        json.dump(report, fh, indent=2, sort_keys=True)
        fh.write("\n")
    lines = ["# Design gate report", "", f"- generated_at: {report['generated_at']}",
             f"- status: {report['status']}", f"- reason: {report.get('reason', '')}", ""]
    if report.get("gates"):
        lines += ["## Executed gates (check: auto)", ""]
        for g in report["gates"]:
            mark = {"pass": "PASS", "fail": "FAIL", "unmapped": "UNMAPPED"}[g["status"]]
            lines.append(f"- [{mark}] `{g['id']}` (severity: {g['severity']}"
                         f"{', blocking' if g['blocking'] else ''}): {g['pass_criteria']}")
            if g["status"] == "fail" and g.get("output_tail"):
                lines += ["", "  ```", *(f"  {l}" for l in g["output_tail"].splitlines()[-15:]), "  ```", ""]
    if report.get("critic_checklist"):
        lines += ["", "## Critic checklist (check: vlm / review — for the QA critic, not executed)", ""]
        lines += [f"- `{c['id']}` ({c['check']}): {c['pass_criteria']}" for c in report["critic_checklist"]]
    if report.get("pack_quality_gates"):
        lines += ["", "## Baseline pack quality gates (verdict checklist for UI-touching PRs)", ""]
        lines += [f"- {q}" for q in report["pack_quality_gates"]]
    summary = (f"autospec-design-gates: {report['status_line']} "
               f"({report['counts']['run']} run, {report['counts']['failed']} failed, "
               f"{report['counts']['unmapped']} unmapped)")
    lines += ["", f"`{summary}`", ""]
    with open(md_out, "w") as fh:
        fh.write("\n".join(lines))
    print(summary)
    sys.exit(exit_code)


def skip(reason):
    finish({
        "version": 1, "generated_at": now_iso(), "status": "skipped", "status_line": "SKIPPED",
        "reason": reason, "gates": [], "critic_checklist": [], "pack_quality_gates": [],
        "counts": {"run": 0, "failed": 0, "unmapped": 0},
    }, 0)


if not os.path.isfile(config_path):
    skip("no-config")

try:
    import yaml
except Exception as exc:
    print(f"autospec-design-gates: python yaml module is required: {exc}", file=sys.stderr)
    sys.exit(2)

with open(config_path) as fh:
    cfg = yaml.safe_load(fh) or {}
if not isinstance(cfg, dict):
    print("autospec-design-gates: config must be a mapping", file=sys.stderr)
    sys.exit(2)


def glob_to_regex(pattern):
    # ** matches any path segment sequence, * matches within a segment.
    out = []
    i = 0
    while i < len(pattern):
        ch = pattern[i]
        if ch == "*":
            if pattern[i:i + 3] == "**/":
                out.append(r"(?:[^/]+/)*")
                i += 3
                continue
            if pattern[i:i + 2] == "**":
                out.append(r".*")
                i += 2
                continue
            out.append(r"[^/]*")
            i += 1
            continue
        out.append(re.escape(ch))
        i += 1
    return re.compile("^" + "".join(out) + "$")


ui_paths = [p for p in (cfg.get("ui_paths") or []) if isinstance(p, str)]
if changed_path and ui_paths:
    with open(changed_path) as fh:
        changed = [l.strip().lstrip("./") for l in fh if l.strip()]
    regexes = [glob_to_regex(p) for p in ui_paths]
    if not any(rx.match(f) for f in changed for rx in regexes):
        skip("skipped_not_ui")

rules_file = cfg.get("rules_file", "")
if not isinstance(rules_file, str) or not rules_file:
    print("autospec-design-gates: config is missing rules_file", file=sys.stderr)
    sys.exit(2)
if not os.path.isabs(rules_file):
    rules_file = os.path.join(repo_root, rules_file)
if not os.path.isfile(rules_file):
    print(f"autospec-design-gates: rules_file not found: {rules_file}", file=sys.stderr)
    sys.exit(2)

with open(rules_file) as fh:
    registry = yaml.safe_load(fh) or {}
rules = [r for r in (registry.get("rules") or []) if isinstance(r, dict) and r.get("id")]

gates_cfg = cfg.get("gates") or {}
if not isinstance(gates_cfg, dict):
    print("autospec-design-gates: gates must be a mapping of rule-id -> {command}", file=sys.stderr)
    sys.exit(2)
unknown = sorted(set(gates_cfg) - {r["id"] for r in rules})
if unknown:
    print(f"autospec-design-gates: gates reference unknown rule ids: {', '.join(unknown)}",
          file=sys.stderr)
    sys.exit(2)

pack_quality_gates = []
pack_file = cfg.get("pack_file", "")
if isinstance(pack_file, str) and pack_file:
    if not os.path.isabs(pack_file):
        pack_file = os.path.join(repo_root, pack_file)
    if not os.path.isfile(pack_file):
        print(f"autospec-design-gates: pack_file not found: {pack_file}", file=sys.stderr)
        sys.exit(2)
    with open(pack_file) as fh:
        pack = json.load(fh)
    pack_quality_gates = [q for q in (pack.get("qualityGates") or []) if isinstance(q, str)]

gate_results, critic_checklist = [], []
run = failed = unmapped = 0
blocking_failure = False
for rule in rules:
    rid = rule["id"]
    check = str(rule.get("check", "review"))
    severity = str(rule.get("severity", "minor"))
    pass_criteria = str(rule.get("pass", "")).strip()
    if check != "auto":
        critic_checklist.append({"id": rid, "check": check, "severity": severity,
                                 "pass_criteria": pass_criteria})
        continue
    mapped = gates_cfg.get(rid)
    blocking = severity == "blocker"
    if isinstance(mapped, dict) and isinstance(mapped.get("blocking"), bool):
        blocking = mapped["blocking"]
    entry = {"id": rid, "severity": severity, "blocking": blocking,
             "pass_criteria": pass_criteria, "tool_hint": str(rule.get("tool", ""))}
    if not isinstance(mapped, dict) or not isinstance(mapped.get("command"), str):
        unmapped += 1
        entry["status"] = "unmapped"
        if strict and severity == "blocker":
            blocking_failure = True
        gate_results.append(entry)
        continue
    run += 1
    proc = subprocess.run(["bash", "-c", mapped["command"]], cwd=repo_root,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    entry["command"] = mapped["command"]
    entry["exit_code"] = proc.returncode
    entry["output_tail"] = (proc.stdout or "")[-2000:]
    if proc.returncode == 0:
        entry["status"] = "pass"
    else:
        entry["status"] = "fail"
        failed += 1
        if blocking:
            blocking_failure = True
    gate_results.append(entry)

status = "fail" if blocking_failure else "pass"
finish({
    "version": 1, "generated_at": now_iso(), "status": status,
    "status_line": "FAIL" if blocking_failure else "PASS",
    "reason": "strict-unmapped-blocker" if (blocking_failure and failed == 0) else "",
    "config_path": config_path, "rules_file": rules_file,
    "gates": gate_results, "critic_checklist": critic_checklist,
    "pack_quality_gates": pack_quality_gates,
    "counts": {"run": run, "failed": failed, "unmapped": unmapped},
}, 1 if blocking_failure else 0)
PY
