#!/usr/bin/env bash
# scripts/ui-evidence-gates.sh — run every runtime UI gate and merge the results.
#
# Why this exists: the five gates shipped over four merges with nothing invoking them. They
# lived as prose steps 0b–0f in the autospec-qa accessibility-and-responsive cluster, each
# with its own CLI, its own JSON shape and its own exit convention, and an operator (or the
# QA agent) was expected to run five commands and reconcile five reports by hand. Steps that
# take five manual invocations get run once, at the start, and then not again.
#
# Modelled on autospec-design-gates.sh: one report, one authoritative status line.
#
# The exit codes carry the distinction the cluster doc insists on — an absent browser is not
# a passing grade:
#
#   0  PASS      every gate ran and reported nothing
#   1  FAIL      at least one gate reported findings
#   2  UNKNOWN   at least one gate could not run, and none reported findings
#
# Findings outrank unknown. A missing browser on one gate must not mask a real defect found
# by another.
set -u

BASE_URL=""
ROUTES=""
GATES_DIR="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"
REPORT_DIR=".autospec/reports"
MANIFEST=".autospec/ui-test-hooks.json"
BASELINE_DIR=".autospec/a11y-baselines"
UPDATE_BASELINES=0

usage() {
  cat <<'EOF'
Usage:
  ui-evidence-gates.sh --base-url <url> [--routes "/ /runs ..."]
                       [--gates-dir <dir>] [--report-dir <dir>]
                       [--manifest <path>] [--baseline-dir <dir>]
                       [--update-baselines]

Runs, in order:
  motion      ui-motion-evidence.mjs       does the UI move, and stop when asked
  device      ui-device-evidence.mjs       real device profiles, 320px reflow, 200% zoom
  keyboard    ui-keyboard-evidence.mjs     traps, focus visibility, focus order
  liveregion  ui-liveregion-evidence.mjs   announcements, induced and declared
  a11y        ui-a11y-baseline.mjs         accessibility-tree regressions

Writes:
  <report-dir>/ui-evidence-gates.json

Final stdout line (parse this, not the exit code alone):
  ui-evidence-gates: PASS|FAIL|UNKNOWN (<ran> ran, <failed> with findings, <unknown> unknown)
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --base-url)         BASE_URL="${2:-}"; shift 2 ;;
    --routes)           ROUTES="${2:-}"; shift 2 ;;
    --gates-dir)        GATES_DIR="${2:-}"; shift 2 ;;
    --report-dir)       REPORT_DIR="${2:-}"; shift 2 ;;
    --manifest)         MANIFEST="${2:-}"; shift 2 ;;
    --baseline-dir)     BASELINE_DIR="${2:-}"; shift 2 ;;
    --update-baselines) UPDATE_BASELINES=1; shift ;;
    -h|--help)          usage; exit 0 ;;
    *) echo "ui-evidence-gates: unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

if [ -z "$BASE_URL" ]; then
  echo "ui-evidence-gates: --base-url is required" >&2
  usage >&2
  exit 64
fi
[ -n "$ROUTES" ] || ROUTES="/"

mkdir -p "$REPORT_DIR"
REPORT="$REPORT_DIR/ui-evidence-gates.json"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ran=0
failed=0
unknown=0

# run_gate NAME SCRIPT [extra args…]
#
# Every gate is run even after an earlier one fails. Stopping at the first failure would hide
# the rest, and an operator would fix one defect per run.
run_gate() {
  name="$1"; script="$2"; shift 2
  path="$GATES_DIR/$script"

  if [ ! -f "$path" ]; then
    # Not skipped: a gate that is not installed answered none of its questions, and calling
    # that a pass is how a partial install reads as a clean bill of health.
    printf '%s\n' "ui-evidence-gates: $name is not installed at $path" > "$WORK/$name.out"
    echo "not_installed" > "$WORK/$name.status"
    unknown=$((unknown + 1))
    return
  fi

  set +e
  node "$path" "$@" > "$WORK/$name.out" 2>&1
  code=$?
  set -e

  # A gate can exit 0 having verified nothing — liveregion does exactly that when every route
  # is server-rendered. Reporting that as "ok" is the silent gap this whole tier exists to
  # close, so a report declaring `measured: 0` is called out rather than counted as clean.
  measured=""
  for candidate in "$REPORT_DIR/$name-evidence.json" "$REPORT_DIR/$name-baseline.json"; do
    [ -f "$candidate" ] || continue
    measured="$(python3 -c "
import json,sys
try:
    d = json.load(open(sys.argv[1]))
except Exception:
    raise SystemExit
m = d.get('measured')
if m is not None:
    print(m)
" "$candidate" 2>/dev/null)"
    [ -n "$measured" ] && break
  done

  case "$code" in
    0) if [ "${measured:-x}" = "0" ]; then
         echo "unmeasured" > "$WORK/$name.status"; unknown=$((unknown + 1))
       else
         echo "clean" > "$WORK/$name.status"; ran=$((ran + 1))
       fi ;;
    3) echo "unknown"  > "$WORK/$name.status"; unknown=$((unknown + 1)) ;;
    *) echo "findings" > "$WORK/$name.status"; ran=$((ran + 1)); failed=$((failed + 1)) ;;
  esac
}

# shellcheck disable=SC2086
run_gate motion     ui-motion-evidence.mjs \
  --base-url "$BASE_URL" --routes $ROUTES --json "$REPORT_DIR/motion-evidence.json"
# shellcheck disable=SC2086
run_gate device     ui-device-evidence.mjs \
  --base-url "$BASE_URL" --routes $ROUTES --json "$REPORT_DIR/device-evidence.json"
# shellcheck disable=SC2086
run_gate keyboard   ui-keyboard-evidence.mjs \
  --base-url "$BASE_URL" --routes $ROUTES --json "$REPORT_DIR/keyboard-evidence.json"
# shellcheck disable=SC2086
run_gate liveregion ui-liveregion-evidence.mjs \
  --base-url "$BASE_URL" --routes $ROUTES --manifest "$MANIFEST" \
  --json "$REPORT_DIR/liveregion-evidence.json"

a11y_args="--base-url $BASE_URL --routes $ROUTES --baseline-dir $BASELINE_DIR --json $REPORT_DIR/a11y-baseline.json"
[ "$UPDATE_BASELINES" -eq 1 ] && a11y_args="$a11y_args --update"
# shellcheck disable=SC2086
run_gate a11y       ui-a11y-baseline.mjs $a11y_args

# Findings outrank unknown: a browser missing for one gate must not mask a defect another
# gate actually found.
if [ "$failed" -gt 0 ]; then
  overall="FAIL"; exit_code=1
elif [ "$unknown" -gt 0 ]; then
  overall="UNKNOWN"; exit_code=2
else
  overall="PASS"; exit_code=0
fi

python3 - "$WORK" "$REPORT" "$overall" "$ran" "$failed" "$unknown" "$BASE_URL" "$ROUTES" <<'PY'
import json, os, sys

work, report, overall, ran, failed, unknown, base_url, routes = sys.argv[1:9]
gates = []
for name in ("motion", "device", "keyboard", "liveregion", "a11y"):
    status_file = os.path.join(work, name + ".status")
    out_file = os.path.join(work, name + ".out")
    if not os.path.exists(status_file):
        continue
    status = open(status_file).read().strip()
    output = open(out_file).read() if os.path.exists(out_file) else ""
    gates.append({
        "gate": name,
        "status": status,
        # Kept whole rather than summarised: the per-gate detail lines are the finding, and a
        # runner that drops them makes the operator re-run each gate by hand to see why.
        "output": output.strip(),
    })

json.dump({
    "schema": 1,
    "status": overall,
    "base_url": base_url,
    "routes": routes.split(),
    "ran": int(ran),
    "with_findings": int(failed),
    "unknown": int(unknown),
    "gates": gates,
}, open(report, "w"), indent=2)
open(report, "a").write("\n")
PY

for name in motion device keyboard liveregion a11y; do
  [ -f "$WORK/$name.status" ] || continue
  status="$(cat "$WORK/$name.status")"
  case "$status" in
    clean)         printf 'ok %s\n' "$name" ;;
    unmeasured)    printf '%s: UNKNOWN — the gate ran and verified nothing\n' "$name"
                   sed 's/^/  /' "$WORK/$name.out" ;;
    findings)      printf '%s: findings\n' "$name"; sed 's/^/  /' "$WORK/$name.out" ;;
    unknown)       printf '%s: UNKNOWN — the gate could not collect evidence\n' "$name"
                   sed 's/^/  /' "$WORK/$name.out" ;;
    not_installed) printf '%s: UNKNOWN — not installed\n' "$name"
                   sed 's/^/  /' "$WORK/$name.out" ;;
  esac
done

echo "report: $REPORT"
# Last line on purpose, so a pipeline can tail -1 rather than parse the whole run.
echo "ui-evidence-gates: $overall ($ran ran, $failed with findings, $unknown unknown)"
exit "$exit_code"
