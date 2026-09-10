#!/usr/bin/env bats
# V54 Multi-repo portfolio read-only planning (issue #3431).
#
# The gate is the Rust module itself: the schema, digest, scope resolution and
# zero-mutation proof are exercised by `cargo test -p autospec-cli --bin autospec`
# (autospec-cli is a binary-only crate, so the harness is `--bin`, not `--lib`),
# plus static guarantees read straight out of the source: the planning path contains
# no write call at all, and every stable error code maps to a distinct exit value that
# the operator document reproduces row for row.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
PORTFOLIO_SRC="crates/autospec-cli/src/commands/managed_project/portfolio.rs"
PORTFOLIO_DIR="crates/autospec-cli/src/commands/managed_project/portfolio"
DRY_RUN_SRC="crates/autospec-cli/src/commands/managed_project/portfolio/dry_run.rs"
MANIFEST_SRC="crates/autospec-cli/src/commands/managed_project/portfolio/manifest.rs"
FACTS_SRC="crates/autospec-cli/src/commands/managed_project/portfolio/manifest/facts.rs"
GRAPH_SRC="crates/autospec-cli/src/commands/managed_project/portfolio/manifest/graph.rs"
MANIFEST_TESTS_SRC="crates/autospec-cli/src/commands/managed_project/portfolio/manifest/tests.rs"
REJECTION_TESTS_SRC="crates/autospec-cli/src/commands/managed_project/portfolio/manifest/rejections.rs"
PLAN_DOC="docs/managed-project-portfolio-plan.md"

# Compile the test binary once for the file so each @test below is a lookup, not a build.
setup_file() {
  cd "$REPO_ROOT"
  if [ -z "${SKIP_CARGO_BUILD_FOR_BATS:-}" ]; then
    cargo build -q -p autospec-cli --tests
  fi
}

# Restarted before every @test so each block can assert how many Rust tests it needed.
setup() {
  RUST_TESTS_RAN=0
}

# Runs one exact Rust test in the autospec binary and counts it on success.
#
# A renamed or deleted test matches nothing and cargo still exits 0, so the pass count
# is checked here, and the per-block count is asserted at the end of every @test below.
run_rust() {
  run bash -c 'cd "$1" && cargo test -q -p autospec-cli --bin autospec -- "$2" -- --exact' \
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

@test "v54 freezes a portfolio plan whose canonical digest is order-insensitive" {
  cd "$REPO_ROOT"
  run_rust commands::managed_project::portfolio::manifest::tests::plan_digest_is_stable_and_order_insensitive
  run_rust commands::managed_project::portfolio::manifest::tests::freeze_stores_canonical_identity_and_sorted_repositories
  [ "$RUST_TESTS_RAN" -eq 2 ]
}

@test "v54 digest is content-addressed: tampering breaks it, quoting stays canonical" {
  cd "$REPO_ROOT"
  run_rust commands::managed_project::portfolio::manifest::tests::plan_digest_covers_revision_capability_and_edges
  run_rust commands::managed_project::portfolio::manifest::tests::tampering_with_a_frozen_plan_breaks_its_digest
  run_rust commands::managed_project::portfolio::manifest::tests::canonical_yaml_is_fully_quoted_and_digest_bearing
  run_rust commands::managed_project::portfolio::manifest::tests::an_undeclared_primary_scope_renders_as_yaml_null

  # The schema and digest namespace constants are the contract downstream issues build
  # on, so pin them here as well as in Rust.
  grep -qF 'PORTFOLIO_PLAN_SCHEMA: &str = "autospec.portfolio-plan.v1"' "$MANIFEST_SRC"
  grep -qF 'DIGEST_NAMESPACE: &[u8] = b"autospec.portfolio-plan.digest.v1"' "$MANIFEST_SRC"
  [ "$RUST_TESTS_RAN" -eq 4 ]
}

@test "v54 rejects three invalid dependency graphs with three distinct exit codes" {
  cd "$REPO_ROOT"

  # Case 1: dependency cycle.
  run_rust commands::managed_project::portfolio::manifest::tests::rejections::rejects_a_dependency_cycle
  # Case 2: self dependency.
  run_rust commands::managed_project::portfolio::manifest::tests::rejections::rejects_a_self_dependency
  # Case 3: edge to a reference that is not in the plan.
  run_rust commands::managed_project::portfolio::manifest::tests::rejections::rejects_an_edge_pointing_at_nothing_in_the_plan

  # Each case maps to its own exit code, written out per variant.
  grep -q 'Self::DependencyCycle => 35' "$MANIFEST_SRC"
  grep -q 'Self::EdgeSelfDependency => 32' "$MANIFEST_SRC"
  grep -q 'Self::EdgeReferenceMissing => 33' "$MANIFEST_SRC"

  # The graph rules are their own module, and the cycle walk is depth-first with an
  # explicit in-progress marker rather than a node-count heuristic.
  grep -q 'fn detect_cycle' "$GRAPH_SRC"
  grep -q 'const VISITING' "$GRAPH_SRC"
  [ "$RUST_TESTS_RAN" -eq 3 ]
}

@test "v54 refuses duplicate edges, cross-repository local parents and a bad schema" {
  cd "$REPO_ROOT"
  run_rust commands::managed_project::portfolio::manifest::tests::rejections::rejects_the_same_edge_declared_twice
  run_rust commands::managed_project::portfolio::manifest::tests::rejections::rejects_a_local_parent_hosted_by_another_repository
  run_rust commands::managed_project::portfolio::manifest::tests::rejections::rejects_an_unsupported_schema
  run_rust commands::managed_project::portfolio::manifest::tests::execution_order_runs_parents_before_dependents

  grep -q 'Self::EdgeDuplicate => 31' "$MANIFEST_SRC"
  grep -q 'Self::LocalParentCrossRepository => 34' "$MANIFEST_SRC"
  grep -q 'Self::SchemaUnsupported => 20' "$MANIFEST_SRC"

  # The rejection suite is a child module of the manifest tests, so the fixtures live
  # in one place; pin the wiring a rename cannot silently drop.
  grep -q 'mod rejections;' "$MANIFEST_TESTS_SRC"
  [ -f "$REJECTION_TESTS_SRC" ]
  [ "$RUST_TESTS_RAN" -eq 4 ]
}

@test "v54 capability facts are tri-state with distinct refusal codes" {
  cd "$REPO_ROOT"

  # Unknown means never probed; unavailable means probed and refused. The two produce
  # different codes so an operator can tell a stale probe from a missing capability.
  run_rust commands::managed_project::portfolio::manifest::tests::rejections::unknown_and_unavailable_capabilities_are_distinct_refusals

  # 26 and 27, distinct from each other.
  grep -q 'Self::RepositoryCapabilityUnknown => 26' "$MANIFEST_SRC"
  grep -q 'Self::RepositoryCapabilityUnavailable => 27' "$MANIFEST_SRC"

  # The three states are exhaustive, so no probe result silently means "fine".
  grep -q 'Self::Available => "available"' "$FACTS_SRC"
  grep -q 'Self::Unavailable => "unavailable"' "$FACTS_SRC"
  grep -q 'Self::Unknown => "unknown"' "$FACTS_SRC"
  [ "$RUST_TESTS_RAN" -eq 1 ]
}

@test "v54 refuses a plan with no owner, an unknown owner, or malformed members" {
  cd "$REPO_ROOT"
  run_rust commands::managed_project::portfolio::manifest::tests::rejections::rejects_missing_owner_unknown_owner_and_empty_repository_set
  run_rust commands::managed_project::portfolio::manifest::tests::rejections::rejects_malformed_or_duplicate_repositories_and_items

  grep -q 'Self::OwnerMissing => 21' "$MANIFEST_SRC"
  grep -q 'Self::OwnerInvalid => 22' "$MANIFEST_SRC"
  grep -q 'Self::PortfolioSetEmpty => 23' "$MANIFEST_SRC"
  grep -q 'Self::RepositoryInvalid => 24' "$MANIFEST_SRC"
  grep -q 'Self::RepositoryDuplicate => 25' "$MANIFEST_SRC"
  grep -q 'Self::ItemKeyInvalid => 28' "$MANIFEST_SRC"
  grep -q 'Self::ItemKeyDuplicate => 29' "$MANIFEST_SRC"
  grep -q 'Self::ItemRepositoryUndeclared => 30' "$MANIFEST_SRC"
  [ "$RUST_TESTS_RAN" -eq 2 ]
}

@test "v54 dry run proves zero durable, remote and filesystem mutations" {
  cd "$REPO_ROOT"

  run_rust commands::managed_project::portfolio::tests::a_dry_run_certifies_zero_mutations_over_a_real_tree
  run_rust commands::managed_project::portfolio::tests::an_in_memory_run_has_nothing_to_check
  run_rust commands::managed_project::portfolio::tests::a_ledger_that_counted_anything_fails_the_zero_check

  # The guarantee is structural: the planning path never issues a write. Only the test
  # fixtures build a journal tree, and only inside the cfg(test) modules tests.rs and
  # rejections.rs.
  run grep -rn "fs::write\|File::create\|fs::remove\|fs::rename\|Command::new\|create_dir" \
    "$PORTFOLIO_SRC" "$PORTFOLIO_DIR" --include='*.rs' \
    --exclude='tests.rs' --exclude='rejections.rs'
  [ "$status" -eq 1 ]

  # The zero-mutation proof lives in its own module and is re-exported flat.
  grep -q 'pub fn validate_plan_dry_run' "$DRY_RUN_SRC"
  grep -q 'pub use self::dry_run::' "$PORTFOLIO_SRC"

  # The mutation counters are incremented nowhere outside the test that exercises them.
  run grep -rn "record_durable()\|record_remote()" \
    "$PORTFOLIO_SRC" "$PORTFOLIO_DIR" --include='*.rs' \
    --exclude='tests.rs' --exclude='rejections.rs'
  [ "$status" -eq 1 ]
  [ "$RUST_TESTS_RAN" -eq 3 ]
}

@test "v54 dry run refuses a missing journal, an invalid plan and a mutated tree" {
  cd "$REPO_ROOT"

  # An absent journal is refused rather than reported as an empty success.
  run_rust commands::managed_project::portfolio::tests::a_dry_run_refuses_a_journal_that_is_not_there

  # An invalid plan never reaches the witness.
  run_rust commands::managed_project::portfolio::tests::a_dry_run_rejects_an_invalid_plan_before_reporting

  # The witness is a real witness: created, modified and removed files are all reported.
  run_rust commands::managed_project::portfolio::tests::the_witness_notices_every_way_a_tree_can_change
  [ "$RUST_TESTS_RAN" -eq 3 ]
}

@test "v54 primary scope is derived, declared, or refused - never guessed" {
  cd "$REPO_ROOT"

  run_rust commands::managed_project::portfolio::tests::one_host_derives_its_product_as_the_primary_scope
  run_rust commands::managed_project::portfolio::tests::an_explicit_spec_portfolio_selector_beats_derivation
  run_rust commands::managed_project::portfolio::tests::several_hosts_without_a_declaration_are_ambiguous_not_guessed
  run_rust commands::managed_project::portfolio::tests::a_declared_product_that_hosts_no_item_is_refused
  run_rust commands::managed_project::portfolio::tests::a_plan_with_nothing_to_build_declares_no_scope

  grep -q 'Self::PrimaryScopeUndeclared => 40' "$PORTFOLIO_SRC"
  grep -q 'Self::PrimaryScopeAmbiguous => 41' "$PORTFOLIO_SRC"
  grep -q 'Self::PrimaryScopeUnknown => 42' "$PORTFOLIO_SRC"
  [ "$RUST_TESTS_RAN" -eq 5 ]
}

@test "v54 operator document reproduces the exit code table exactly" {
  cd "$REPO_ROOT"

  run_rust commands::managed_project::portfolio::tests::the_documented_exit_code_table_matches_the_code_table

  # The document itself must exist, name the schema, and carry all twenty rows, so a
  # deleted table cannot make the Rust comparison vacuously pass.
  [ -f "$PLAN_DOC" ]
  grep -q 'autospec.portfolio-plan.v1' "$PLAN_DOC"
  rows="$(awk -F'|' '/^\| `[A-Z_]+` \| *[0-9]+ *\|/ { n++ } END { print n + 0 }' "$PLAN_DOC")"
  [ "$rows" -eq 20 ]
  [ "$RUST_TESTS_RAN" -eq 1 ]
}

@test "v54 exit codes stay distinct across both violation families" {
  cd "$REPO_ROOT"
  run_rust commands::managed_project::portfolio::tests::every_documented_exit_code_is_distinct
  [ "$RUST_TESTS_RAN" -eq 1 ]
}

# Builds a minimal lock-step trio (SKILL.md + opencode/agent.md + codex/prompt.md)
# for one workflow skill under $1. $3 = wrong puts `gh issue create` ahead of
# `portfolio apply` in the shared body; good puts the transaction first.
write_workflow_trio() {
  local root="$1" skill="$2" order="$3"
  local dir="$root/skills/$skill"
  mkdir -p "$dir/codex" "$dir/opencode"
  if [ "$order" = "wrong" ]; then
    BODY="File the issues first with gh issue create --label needs-classify.
Then provision the primary portfolio with autospec portfolio apply --manifest planned.yml."
  else
    BODY="Provision the primary portfolio first with autospec portfolio apply --manifest planned.yml.
Then file the issues with gh issue create --label needs-classify."
  fi
  printf -- '---\nname: %s\n---\n%s\n' "$skill" "$BODY" > "$dir/SKILL.md"
  printf -- '---\nname: %s\nmode: primary\n---\n%s\n' "$skill" "$BODY" > "$dir/opencode/agent.md"
  printf '%s\n' "$BODY" > "$dir/codex/prompt.md"
}

@test "v54 workflow validator enforces portfolio-first ordering on every decomposition path" {
  cd "$REPO_ROOT"
  local script="$REPO_ROOT/scripts/autospec-validate-state.sh"
  local skill root

  # Uniform enforcement: the validator names every workflow entry point in one
  # list, so no definition path can opt out of the order and lock-step checks.
  grep -qF '"autospec", "autospec-define", "autospec-split", "autospec-explore", "autospec-run"' "$script"

  # A wrong-order body fails on every decomposition path, not just one skill.
  for skill in autospec autospec-define autospec-split autospec-explore autospec-run; do
    root="$BATS_TEST_TMPDIR/order-$skill"
    write_workflow_trio "$root" "$skill" wrong
    run bash "$script" --repo-root "$root"
    if [ "$status" -ne 1 ]; then
      echo "$skill: wrong-order fixture unexpectedly passed" >&3
      return 1
    fi
    grep -qF 'portfolio provisioning after first gh issue create' \
      "$root/.autospec/reports/state-validation.md"
  done

  # The same body in the right order is clean.
  root="$BATS_TEST_TMPDIR/order-good"
  write_workflow_trio "$root" "autospec-define" good
  run bash "$script" --repo-root "$root"
  [ "$status" -eq 0 ]
  [[ "$output" == *"state validation: pass"* ]]
  [ "$RUST_TESTS_RAN" -eq 0 ]
}

@test "v54 workflow validator fails a generated lock-step body that diverges" {
  cd "$REPO_ROOT"
  local script="$REPO_ROOT/scripts/autospec-validate-state.sh"
  local root="$BATS_TEST_TMPDIR/lockstep-diverged"

  write_workflow_trio "$root" "autospec-split" good
  echo "divergent codex-only line" >> "$root/skills/autospec-split/codex/prompt.md"
  run bash "$script" --repo-root "$root"
  [ "$status" -eq 1 ]
  grep -qF 'lock-step body diverges from codex/prompt.md' \
    "$root/.autospec/reports/state-validation.md"

  write_workflow_trio "$BATS_TEST_TMPDIR/lockstep-diverged-2" "autospec-run" good
  echo "divergent opencode-only line" >> \
    "$BATS_TEST_TMPDIR/lockstep-diverged-2/skills/autospec-run/opencode/agent.md"
  run bash "$script" --repo-root "$BATS_TEST_TMPDIR/lockstep-diverged-2"
  [ "$status" -eq 1 ]
  grep -qF 'lock-step body diverges from opencode/agent.md' \
    "$BATS_TEST_TMPDIR/lockstep-diverged-2/.autospec/reports/state-validation.md"
  [ "$RUST_TESTS_RAN" -eq 0 ]
}

@test "v54 dry-run report proves zero mutation and keeps capabilities tri-state" {
  cd "$REPO_ROOT"
  local script="$REPO_ROOT/scripts/autospec-validate-state.sh"

  # A dry-run report by itself is clean: plan shape plus tri-state capabilities,
  # no transaction behind it.
  local clean="$BATS_TEST_TMPDIR/dry-clean"
  mkdir -p "$clean/.autospec/state"
  cat > "$clean/.autospec/state/portfolio-dry-run.json" <<'JSON'
{
  "schema": "autospec.portfolio-plan.v1",
  "dry_run": true,
  "portfolio_id": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "capabilities": {"projects": "unavailable", "issue_create": "unknown", "coord_ref": "verified"}
}
JSON
  run bash "$script" --repo-root "$clean"
  [ "$status" -eq 0 ]
  [[ "$output" == *"state validation: pass"* ]]

  # The same dry run with a transaction left behind for its portfolio_id is a
  # mutation, and a dry run must never claim one.
  local dirty="$BATS_TEST_TMPDIR/dirty"
  mkdir -p "$dirty/.autospec/state"
  cp "$clean/.autospec/state/portfolio-dry-run.json" "$dirty/.autospec/state/"
  cat > "$dirty/.autospec/state/portfolio-transaction.json" <<'JSON'
{
  "schema": "autospec.portfolio-transaction.v1",
  "portfolio_id": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "project_owner": "org",
  "plan_digest": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210",
  "state": "blocked"
}
JSON
  run bash "$script" --repo-root "$dirty"
  [ "$status" -eq 1 ]
  grep -qF 'dry run left a portfolio transaction' \
    "$dirty/.autospec/reports/state-validation.md"

  # A capability outside the tri-state set is a guessed permission, refused.
  local badcap="$BATS_TEST_TMPDIR/badcap"
  mkdir -p "$badcap/.autospec/state"
  cat > "$badcap/.autospec/state/portfolio-dry-run.json" <<'JSON'
{
  "schema": "autospec.portfolio-plan.v1",
  "dry_run": true,
  "portfolio_id": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "capabilities": {"projects": "granted"}
}
JSON
  run bash "$script" --repo-root "$badcap"
  [ "$status" -eq 1 ]
  grep -qF 'must be verified, unavailable or unknown' \
    "$badcap/.autospec/reports/state-validation.md"
  [ "$RUST_TESTS_RAN" -eq 0 ]
}
