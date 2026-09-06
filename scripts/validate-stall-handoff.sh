#!/usr/bin/env bash
# Validation for the stall handoff module (#3563).
#
# The bugs this issue describes were structural, not logical: evidence captured
# into a local variable and dropped, the queue told "no diff" when a diff
# existed, the retry policy "same model, one retry" with no roster behind it.
# Each of those is checkable by reading the source, so this script does that and
# fails the build if the shape regresses.
#
# Usage: scripts/validate-stall-handoff.sh [--quiet]
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

QUIET=0
[ "${1:-}" = "--quiet" ] && QUIET=1

failures=0

fail() {
  failures=$((failures + 1))
  printf 'stall-handoff: FAIL: %s\n' "$*"
}

note() {
  [ "$QUIET" -eq 1 ] || printf 'stall-handoff: %s\n' "$*"
}

require_file() {
  [ -f "$1" ] || fail "missing $1"
}

require_text() {
  local path="$1" pattern="$2" what="$3"
  [ -f "$path" ] || { fail "missing $path"; return; }
  grep -Eq -- "$pattern" "$path" || fail "$path does not contain /$pattern/ ($what)"
}

forbid_text() {
  local path="$1" pattern="$2" what="$3"
  [ -f "$path" ] || return
  if grep -Eq -- "$pattern" "$path"; then
    fail "$path contains /$pattern/ ($what)"
  fi
}

STALL_DIR="crates/autospec-core/src/stall"
TESTS_DIR="crates/autospec-core/tests"
MAX_LOC=600

# --- structure -------------------------------------------------------------

require_file "$STALL_DIR/mod.rs"
require_file "$STALL_DIR/lease.rs"
require_file "$STALL_DIR/liveness.rs"
require_file "$STALL_DIR/partial_work.rs"
require_file "$STALL_DIR/attempts.rs"
require_file "$STALL_DIR/tracker.rs"
require_file "$STALL_DIR/release.rs"
require_file "$STALL_DIR/note.rs"
require_text crates/autospec-core/src/lib.rs '^pub mod stall;' "module is reachable from the crate root"

for f in "$STALL_DIR"/*.rs "$TESTS_DIR"/stall_*.rs; do
  [ -f "$f" ] || continue
  lines="$(wc -l < "$f" | tr -d ' ')"
  [ "$lines" -le "$MAX_LOC" ] || fail "$f is $lines lines, over the $MAX_LOC ratchet"
done

for suite in stall_lease_liveness stall_release_decisions stall_partial_work_capture stall_release_orchestration; do
  require_file "$TESTS_DIR/$suite.rs"
  require_text "$TESTS_DIR/$suite.rs" '#\[test\]' "at least one test"
done

# --- the tracker stays a trait ---------------------------------------------
#
# The release path must not know it is talking to GitHub: a Jira or local
# tracker has to be able to implement the same interface.

for f in "$STALL_DIR"/*.rs; do
  forbid_text "$f" 'gh (api|issue|pr)' "no shell-outs to the GitHub CLI"
  forbid_text "$f" 'github\.com' "no hardcoded GitHub hosts"
  forbid_text "$f" 'reqwest|hyper::' "no HTTP client: the tracker trait is the seam"
done

require_text "$STALL_DIR/tracker.rs" 'pub trait IssueTracker' "the tracker seam"
require_text "$STALL_DIR/tracker.rs" 'fn escalate_to_spec_repair' "spec-repair handoff on the trait"

# --- evidence is captured before the queue is touched ----------------------
#
# The order is the fix: a tracker outage must never cost the work, and a worktree
# torn down before capture is what produced "no diff, please retry".

# The call site, not the `pub use` that mentions the same identifier.
capture_line="$(grep -n 'capture_partial_work(' "$STALL_DIR/mod.rs" | head -1 | cut -d: -f1 || true)"
release_line="$(grep -n 'release_to_queue' "$STALL_DIR/mod.rs" | head -1 | cut -d: -f1 || true)"
if [ -z "$capture_line" ] || [ -z "$release_line" ]; then
  fail "mod.rs must both capture evidence and release to the queue"
elif [ "$capture_line" -ge "$release_line" ]; then
  fail "mod.rs must capture evidence (line $capture_line) before releasing to the queue (line $release_line)"
fi

# --- the recorded work is the captured work --------------------------------

require_text "$STALL_DIR/mod.rs" 'record\.produced = work\.work_produced' "the handoff records the captured work, not the default"
require_text "$STALL_DIR/mod.rs" 'capture_partial_work' "capture actually runs on the release path"
require_text "$STALL_DIR/partial_work.rs" 'pub trait WorktreeEvidence' "evidence capture stays portable off the local filesystem"
require_text "$STALL_DIR/partial_work.rs" 'AUTOSPEC_GIT_PROGRAM' "the git binary is overridable for containers and remote runners"

# --- retry policy is a roster rotation ------------------------------------

require_text "$STALL_DIR/attempts.rs" 'pub struct ModelRoster' "a roster, not one configured model"
require_text "$STALL_DIR/attempts.rs" 'fn select_next' "rotation picks the next model"
require_text "$STALL_DIR/release.rs" 'AttemptLimit' "the attempt limit is an explicit escalation reason"
require_text "$STALL_DIR/release.rs" 'RotationUnavailable' "a single-model config says rotation is unavailable"

# --- documentation of the new surface -------------------------------------

require_file docs/stall-handoff.md
for var in AUTOSPEC_ISSUE_LEASE_SECS AUTOSPEC_ISSUE_LEASE_RENEW_SECS AUTOSPEC_ISSUE_STALL_SECS \
  AUTOSPEC_STALL_MAX_ATTEMPTS AUTOSPEC_STALL_TRANSCRIPT_TAIL_BYTES AUTOSPEC_GIT_PROGRAM; do
  require_text docs/CONFIG_REFERENCE.md "$var" "the $var environment variable is documented"
done
require_text docs/CONFIG_REFERENCE.md 'stalled-attempts-exhausted' "the escalation label is documented"

[ "$failures" -eq 0 ] || { printf 'stall-handoff: %s finding(s)\n' "$failures"; exit 1; }
note "OK — module shape, capture ordering, tracker neutrality, retry policy, docs"
