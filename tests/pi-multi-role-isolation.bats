#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

# tests/pi-multi-role-isolation.bats — Pi + Qwen3.8 M9 (issue #3324).
#
# Planner, builder and reviewer run as separate sessions: distinct session
# ids, independent worktrees for modifying lanes, read-only parallelism, and
# structured artifact passing between sessions. The gate is the Rust module
# itself (crates/autospec-core/src/aar/isolation.rs) plus the integration
# suite (crates/autospec-core/tests/aar_session_isolation.rs), driven through
# `cargo test` so this file stays deterministic: no clock, no network, no
# model calls.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
ISOLATION_SRC="crates/autospec-core/src/aar/isolation.rs"
ISOLATION_TEST="crates/autospec-core/tests/aar_session_isolation.rs"

# Compile the test binary once for the file so each @test below is a lookup, not a build.
setup_file() {
  cd "$REPO_ROOT"
  if [ -z "${SKIP_CARGO_BUILD_FOR_BATS:-}" ]; then
    cargo build -q -p autospec-core --tests
  fi
}

# Restarted before every @test so each block can assert how many Rust tests it needed.
setup() {
  RUST_TESTS_RAN=0
}

# Runs one exact Rust test from the aar_session_isolation integration suite
# and counts it on success.
#
# A renamed or deleted test matches nothing and cargo still exits 0, so the pass count
# is checked here, and the per-block count is asserted at the end of every @test below.
run_rust() {
  run bash -c 'cd "$1" && cargo test -q -p autospec-core --test aar_session_isolation -- "$2" -- --exact' \
    _ "$REPO_ROOT" "$1"
  if [ "$status" -ne 0 ]; then
    echo "rust test $1 exited $status:" >&3
    echo "$output" >&3
    return 1
  fi
  if [[ "$output" != *"1 passed"* ]]; then
    echo "rust test $1 did not run (renamed or deleted?):" >&3
    echo "$output" >&3
    return 1
  fi
  RUST_TESTS_RAN=$((RUST_TESTS_RAN + 1))
}

@test "AC1: planner, builder and reviewer run as three distinct sessions" {
  cd "$REPO_ROOT"
  run_rust planner_builder_reviewer_run_as_three_distinct_sessions
  run_rust policy_mismatch_fails_closed
  run_rust empty_grant_fields_fail_closed
  run_rust finishing_an_unknown_session_fails_closed
  [ "$RUST_TESTS_RAN" -eq 4 ]
}

@test "AC2: read-only lanes share a worktree in parallel and finish with zero edits" {
  cd "$REPO_ROOT"
  run_rust read_only_sessions_share_a_worktree_in_parallel
  run_rust read_only_session_reporting_edits_fails_closed
  [ "$RUST_TESTS_RAN" -eq 2 ]
}

@test "AC3: modifying sessions get independent worktrees (second writer fails closed)" {
  cd "$REPO_ROOT"
  run_rust two_mutating_sessions_use_two_distinct_worktrees
  run_rust two_mutating_sessions_collide_on_one_worktree_and_fail_closed
  run_rust read_only_sessions_may_join_a_writer_worktree
  run_rust writer_claim_releases_when_the_builder_finishes
  [ "$RUST_TESTS_RAN" -eq 4 ]
}

@test "AC4: reviewer output validates as approve|changes_required|uncertain" {
  cd "$REPO_ROOT"
  run_rust review_verdict_accepts_exactly_three_tokens
  run_rust review_verdict_rejects_everything_else
  [ "$RUST_TESTS_RAN" -eq 2 ]

  # The three verdict tokens are the wire contract, so pin them in source as
  # well as in the Rust suite.
  for token in approve changes_required uncertain; do
    grep -q "$token" "$ISOLATION_SRC"
  done
}

@test "structured artifacts pass between sessions over a JSON wire format" {
  cd "$REPO_ROOT"
  run_rust session_artifacts_roundtrip_the_wire_format
  run_rust session_artifact_wire_format_rejects_garbage
  run_rust session_artifact_validation_refuses_empty_content
  [ "$RUST_TESTS_RAN" -eq 3 ]

  # The wire tag is what a second harness will parse, so it is pinned too.
  grep -q '#\[serde(tag = "kind"' "$ISOLATION_SRC"
}

@test "folded pi session results feed the isolation check" {
  cd "$REPO_ROOT"
  run_rust pi_session_results_fold_into_the_isolation_check
  [ "$RUST_TESTS_RAN" -eq 1 ]
}

@test "the full planner-builder-reviewer run passes end to end" {
  cd "$REPO_ROOT"
  run_rust multi_role_run_end_to_end
  [ "$RUST_TESTS_RAN" -eq 1 ]
}

@test "the whole aar_session_isolation suite is green (17 tests)" {
  cd "$REPO_ROOT"
  run bash -c 'cd "$1" && cargo test -q -p autospec-core --test aar_session_isolation' \
    _ "$REPO_ROOT"
  [ "$status" -eq 0 ]
  [[ "$output" == *"17 passed"* ]]
}

@test "static guarantees: the isolation registry lives in autospec-core, not the CLI" {
  cd "$REPO_ROOT"
  # The decision layer is provider-neutral: the module is in the core crate
  # and re-exported from aar::; the CLI gains nothing here.
  grep -q 'pub mod isolation;' crates/autospec-core/src/aar/mod.rs
  grep -q 'pub use isolation::' crates/autospec-core/src/aar/mod.rs
  grep -q 'IsolationViolation' "$ISOLATION_SRC"
  grep -q 'ReadOnlyBreach' "$ISOLATION_SRC"
  grep -q 'WorktreeCollision' "$ISOLATION_SRC"
  [ -f "$ISOLATION_TEST" ]
}
