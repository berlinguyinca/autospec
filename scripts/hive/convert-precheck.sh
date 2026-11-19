#!/bin/bash
# Cheap "is any conversion work pending?" check for scripts/hive/convert-cron.sh.
#
# The cron fires every 30 minutes (48/day). 1,449 of 1,493 Slurm jobs in 30 days
# ended in seconds having converted nothing: every candidate was already attempted
# or held, yet each pass still allocated 8 CPU / 32G on a compute node first. This
# check decides that WITHOUT compute, so the srun is only requested when work exists.
#
# usage: convert-precheck.sh LLM_ROOT HELD_FILE REPO BASE BRANCH_PREFIX [ISSUE ...]
#   REPO may be empty (PR liveness is then not checked, as `autospec convert` does).
#   Run it from the conversion checkout (the plan reads origin/BASE from cwd).
# exit 0  -> work pending (or undecidable: fail OPEN, never skip real work)
# exit 10 -> idle; the caller must skip the srun
# stdout: one line, "pending: ..." or "idle: ...".
#
# Stage 1 is pure filesystem. Stage 2 is `autospec convert` WITHOUT --apply, the
# pass's own side-effect-free plan: one `git ls-remote` + one `gh pr list`, ~2s CPU
# (network-bound, ~35s wall on hive), niced and timeboxed. It uses the pass's own
# selection (attempted / held / delivered / closed), so this never re-implements it.
set -uo pipefail

LLM=${1:?llm root} HELD=${2:?held file} REPO=${3-} BASE=${4:?base} PREFIX=${5:?branch prefix}
shift 5
PLAN_TIMEOUT=${PLAN_TIMEOUT:-120}

# Stage 1: a candidate with no changes.patch cannot be converted.
have=()
for n in "$@"; do
  for p in "$LLM"/*/out/issue-"$n"/changes.patch; do
    [ -f "$p" ] && { have+=("$n"); break; }
  done
done
if [ "${#have[@]}" -eq 0 ]; then
  echo "idle: none of the ${#} candidate(s) has a changes.patch"
  exit 10
fi

# Stage 2: the pass's own plan.
args=(convert --llm-root "$LLM" --base "$BASE" --held-file "$HELD" --branch-prefix "$PREFIX")
[ -n "$REPO" ] && args+=(--repo "$REPO")
out=$(timeout -k 5 "$PLAN_TIMEOUT" nice autospec "${args[@]}" "${have[@]}" 2>/dev/null)
rc=$?
line=$(printf '%s\n' "$out" | grep -m1 '^conversion pass: examined=')
if [ "$rc" -ne 0 ] || [ -z "$line" ]; then
  echo "pending: plan undecidable (rc=$rc); failing open so no work is skipped"
  exit 0
fi
field() { printf '%s' "$line" | grep -o "$1=[0-9]*" | head -1 | cut -d= -f2; }
fresh=$(field fresh) delivered=$(field delivered) closed=$(field closed)
if [ -z "$fresh" ] || [ -z "$delivered" ] || [ -z "$closed" ]; then
  echo "pending: plan line unparsable; failing open"
  exit 0
fi
# delivered/closed patches are archived by --apply without a gate, so they count too.
if [ $((fresh + delivered + closed)) -eq 0 ]; then
  echo "idle: plan found no fresh, delivered or closed patches (${line#conversion pass: })"
  exit 10
fi
echo "pending: fresh=$fresh delivered=$delivered closed=$closed"
exit 0
