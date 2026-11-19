#!/bin/bash
# Scheduled patch-to-PR conversion (autospec #4584, #4565).
#
# Runs the Rust pass -- `autospec convert --apply` -- rather than reimplementing
# it in shell. Everything it needs is already on the cluster: the binary, the
# patch root, and git access to the remote. `gh` needs a token for PR creation;
# the pass is still useful without one (it selects, applies, gates and pushes),
# so a missing token degrades rather than aborts.
set -uo pipefail
# Resolved before the script cd's into the checkout below.
HERE=$(dirname "$(readlink -f "$0")")
L=${CONVERT_LLM_ROOT:-/quobyte/metabolomicsgrp/it/llm}
LOG=$L/logs/convert-cron.log
LOCK=$L/state/convert.lock
# The gate runs cargo. The rust module only prepends this one directory, and
# sourcing /etc/profile terminates a non-interactive shell on compute nodes,
# so name it directly. Without it every patch is HELD with
# "baseline test count undeterminable" -- the gate cannot run at all, and the
# fail-closed detector correctly refuses rather than passing on a blank gate.
export PATH=/cvmfs/hpc.ucdavis.edu/sw/rust/1.91.0/bin:$L/bin:/cvmfs/hpc.ucdavis.edu/sw/spack/environments/core/view/generic/slurm/bin:$PATH

# Preconditions. A gate that cannot run does not fail honestly -- it produces a
# blank test stage, and the fail-closed baseline check then HELDs every patch
# with "baseline test count undeterminable". 28 patches were held that way by a
# missing cargo before this check existed. Refuse the pass instead: a tool
# absence is an operator problem, not 500 patch verdicts.
for tool in cargo git gh autospec; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "$(date -Is) FATAL: $tool not on PATH -- refusing the pass (a blank gate would HELD every patch)" >> "$LOG"
    exit 1
  }
done

exec 9>"$LOCK" || { echo "$(date -Is) skip: cannot open lock" >> "$LOG"; exit 0; }
flock -n 9 || { echo "$(date -Is) skip: a conversion pass is already running" >> "$LOG"; exit 0; }

# Token: read from the operator-placed secret if present. Never echoed.
TOKFILE=$L/state/secrets/gh.token
if [ -s "$TOKFILE" ]; then
  GH_TOKEN=$(cat "$TOKFILE"); export GH_TOKEN
else
  echo "$(date -Is) NOTE: no gh token at $TOKFILE -- gating and pushing only, no PRs" >> "$LOG"
fi

# The pass needs a repo checkout to apply patches into. The agent base is
# already a clean, packed clone of main and is refreshed by topup; use a
# separate working copy so a conversion never disturbs what agents copy.
WT=$L/convert/repo
mkdir -p "$L/convert"
# git over ssh from cron: no agent, so it must use the account's default key, and
# it must never prompt. Failures used to land only in cron-convert.log as a bare
# "Permission denied (publickey)" with no timestamp, and the script then carried on
# against a stale checkout. Observed 2026-11-04: ONE denial, adjacent to GitHub
# "Connection refused" / "closed by remote host" lines (a GitHub-side ssh blip);
# `ssh -T git@github.com` from the same account authenticates today with
# ~/.ssh/id_ed25519, so the key is valid and the failure was transient. Retry, name
# the key/host/user in the log (fingerprint only, never key material), and skip the
# pass on failure instead of converting against a stale base.
export GIT_SSH_COMMAND="ssh -o BatchMode=yes -o ConnectTimeout=15"
git_retry() { # git_retry ARGS... : 3 tries, stderr kept for the log
  local n err
  for n in 1 2 3; do
    err=$(git "$@" 2>&1) && return 0
    sleep $((n * ${CONVERT_FETCH_SLEEP:-10}))
  done
  {
    echo "$(date -Is) FATAL: git $1 failed 3x as $(id -un)@$(hostname -s) ($(ssh-keygen -lf "$HOME/.ssh/id_ed25519.pub" 2>/dev/null | cut -d' ' -f1-2 | tr '\n' ' '))"
    printf '%s\n' "$err" | sed 's/^/    git: /'
  } >> "$LOG"
  return 1
}
if [ -d "$WT/.git" ]; then
  git_retry -C "$WT" fetch -q --prune origin || exit 1
  git -C "$WT" reset -q --hard origin/main && git -C "$WT" clean -qfd
else
  git_retry clone -q --single-branch --branch main git@github.com:berlinguyinca/autospec.git "$WT" || exit 1
fi

cd "$WT" || exit 1
export CARGO_TARGET_DIR=$L/convert/target
# The GATE reads its own variable, not CARGO_TARGET_DIR, and its default is
# $HOME/.cache/autospec-convert-target. $HOME here is a quota-limited network
# home: the cache reached 13G of a 20G home, cargo then failed with
# "Disk quota exceeded (os error 122)" about 2s into every test stage, and the
# gate -- correctly -- reported "baseline test count undeterminable" and HELD
# the patch. 12 patches were held that way, each one never actually judged.
# Nothing may write to $HOME on this cluster.
export AUTOSPEC_CONVERT_TARGET_DIR=$L/convert/gate-target
START=$(date +%s)
echo "$(date -Is) pass start: base=$(git rev-parse --short HEAD)" >> "$LOG"

# stdbuf does not help a Rust binary's own buffering (#4572); the summary line
# flushes at exit, which is what the log below records.
# WORKAROUND for #4587: selection issues one `git ls-remote` per candidate, so an
# unbounded pass spends its whole period deciding what to do -- observed still
# walking candidates after 340s without reaching a single gate. Bound the batch
# to the newest N patches so each run actually converts something.
# Newest-first is the right bias: recent patches were written against a closer
# base and still apply; older cohorts are conflict-bound (#4577).
# Remove this once selection is O(1) in remote queries.
BATCH_N=${BATCH_N:-4}
CANDIDATES=$(ls -d "$L"/autospec/out/issue-*/ 2>/dev/null \
  | sed -e 's|.*/issue-||' -e 's|/||' | sort -n | tail -n "$BATCH_N" | tr '\n' ' ')
echo "$(date -Is) batch: newest $BATCH_N of $(ls -d "$L"/autospec/out/issue-*/ 2>/dev/null | wc -l) candidates" >> "$LOG"

# The gate compiles and runs the workspace test suite. That is real work, and
# real work does not happen on the submit host: a `cargo test` here spawns a
# dozen rustc/linker processes on a node shared by every user of the cluster,
# and the head node's `ulimit -u` makes such a build fail in ways that read as
# a broken patch rather than a broken environment. Observed directly: a pass
# ran 96 minutes of cargo on login1 and blocked three scheduled passes behind
# its own lock. Send the pass to a compute node; this script stays resident on
# login1 holding the flock and doing nothing but waiting.
#
# Two separate bounds, because they fail differently and must be told apart:
#   -I$ALLOC_WAIT  gives up if the scheduler cannot allocate within the window.
#                  Without it a queued pass holds the lock indefinitely and every
#                  later pass logs "already running" -- indistinguishable from a
#                  healthy busy pass, which is the log ambiguity of #4596.
#   timeout $MAX   bounds the run itself once it has started.
# `9>&-` closes the lock fd for the pass and everything it spawns. flock is held
# for as long as ANY process holds the descriptor, and the pass starts helper
# processes (a stub HTTP server, per-test fixtures) that can outlive it. Observed:
# an orphaned `python3 -m http.server`, its parent dead and its cwd already
# deleted, held this lock for 47 minutes while every scheduled pass logged
# "already running". The lock belongs to this script alone; nothing it launches
# should be able to extend it.
# Skip the allocation when there is nothing to convert. 1,449 of 1,493 jobs in 30
# days ended in seconds having done nothing; the decision is cheap and needs no
# compute (see convert-precheck.sh). It fails OPEN: if it cannot decide, the pass
# runs exactly as before. Set CONVERT_PRECHECK=0 to bypass it.
PRECHECK=${CONVERT_PRECHECK_BIN:-$HERE/convert-precheck.sh}
if [ "${CONVERT_PRECHECK:-1}" != 0 ] && [ -x "$PRECHECK" ]; then
  # shellcheck disable=SC2086  # $CANDIDATES is a space-separated issue list
  VERDICT=$("$PRECHECK" "$L" "$L/autospec/held.jsonl" berlinguyinca/autospec main conv- $CANDIDATES 9>&-)
  if [ $? -eq 10 ]; then
    echo "$(date -Is) pass end SKIPPED, no allocation requested: $VERDICT" >> "$LOG"
    exit 0
  fi
  echo "$(date -Is) precheck: $VERDICT" >> "$LOG"
fi

SRUN_ERR=$L/state/convert-srun.err
: > "$SRUN_ERR"
ALLOC_WAIT=${ALLOC_WAIT:-600}
MAX_RUN=${MAX_RUN:-7200}
timeout -k 30 "$MAX_RUN" \
  srun --quiet -A metabolomicsgrp -q metabolomicsgrp-high-qos -p high \
    -c 8 --mem 32G -t 02:30:00 -J conv-pass -I"$ALLOC_WAIT" \
    autospec convert --apply \
      --llm-root "$L" \
      --repo berlinguyinca/autospec \
      --base main \
      --held-file "$L/autospec/held.jsonl" \
      --branch-prefix conv- $CANDIDATES >> "$LOG" 2>"$SRUN_ERR" 9>&-
RC=$?
ELAPSED=$(( $(date +%s) - START ))
case "$RC" in
  124|137) echo "$(date -Is) pass end TIMEOUT after ${ELAPSED}s (run exceeded MAX_RUN=${MAX_RUN}s) -- lock released" >> "$LOG" ;;
  0)       echo "$(date -Is) pass end rc=0 elapsed=${ELAPSED}s" >> "$LOG" ;;
  *)       if grep -qE "Unable to allocate resources|Requested (node|partition) configuration is not available|resources? (are )?not available|--immediate" "$SRUN_ERR" 2>/dev/null \
     && ! grep -qE "task [0-9]+:" "$SRUN_ERR" 2>/dev/null; then
             echo "$(date -Is) pass end NO ALLOCATION (srun could not place the job within ${ALLOC_WAIT}s) -- the scheduler is full, not a patch defect; lock released" >> "$LOG"
             sed -n "1,3p" "$SRUN_ERR" | sed "s/^/    srun: /" >> "$LOG"
           elif grep -qE "task [0-9]+: (Bus error|Segmentation fault|Killed|Aborted)" "$SRUN_ERR" 2>/dev/null; then
             # The gate died on the execution host. That is not a verdict on any
             # patch -- it is the run failing -- so it must not read like one.
             echo "$(date -Is) pass end GATE CRASHED on the execution host (not a patch defect); lock released" >> "$LOG"
             grep -m2 -E "task [0-9]+:" "$SRUN_ERR" | sed "s/^/    srun: /" >> "$LOG"
           else
             echo "$(date -Is) pass end rc=$RC elapsed=${ELAPSED}s" >> "$LOG"
           fi ;;
esac
# The gate's own stderr still belongs in the log, whichever branch ran.
cat "$SRUN_ERR" >> "$LOG" 2>/dev/null
