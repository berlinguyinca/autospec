# Task 6 Report: Repository Registration and Managed Onboarding

Status: DONE_WITH_CONCERNS

## Commit

- `279778aa` — `feat: register repositories with managed projects`

## Red evidence

- `cargo test -p autospec-cli --test managed_project
  onboard_cli_records_contains_and_only_explicit_creation_records_spawned_from -- --nocapture`
  → exit `101`; the new CLI contract failed before implementation because the
  created/adopted provenance and containment relationships were absent.
- `bats tests/autospec/managed-project-workflows.bats` → exit `1`; the new
  bootstrap ordering and control-plane registration tests failed because no
  post-verification repository registration existed.
- `bats tests/install/project-board-install.bats` → exit `1`; the clean-install
  contract failed because the installed Rust runtime command surface was not
  verified before workflow skill installation.

## Green evidence

- `cargo test -p autospec-cli --test managed_project onboard_ --no-fail-fast`
  → exit `0`; `13 passed`, `0 failed`.
- `bats tests/autospec/managed-project-workflows.bats` → exit `0`; `10 passed`,
  `0 failed`.
- `bats tests/install/project-board-install.bats` → exit `0`; `8 passed`,
  `0 failed`.
- Combined workflow, install, control-plane, block-expansion, and
  installed-runtime regression run → exit `0`; `42 passed`, `0 failed`.
- `cargo check -p autospec-cli` and
  `cargo clippy -p autospec-cli --bin autospec --no-deps` → exit `0`; only
  pre-existing dead-code and unnecessary-cast warnings outside Task 6.
- Targeted `rustfmt --check` with child traversal disabled, `bash -n` for the
  changed Bash entrypoints, `sh -n` for the project installer, trio byte-parity,
  golden regeneration idempotency, and `git diff --check` → exit `0`.

## Files changed

- `crates/autospec-cli/src/commands/managed_project/cli.rs` — added explicit
  `--spawned-from` registration evidence, additive `contains` relationships,
  and `project_url` in onboarding output.
- `crates/autospec-cli/tests/managed_project.rs` — locked adopted-versus-created
  repository provenance through the real journal and CLI boundary.
- `scripts/autospec-control-plane.sh` — verifies repository URL/default branch,
  registers exact slugs, and supplies creation provenance only for repositories
  created by the current bootstrap.
- `skills/autospec{,-define,-split,-project}/` trios — documented verified
  bootstrap registration plus bounded `/autospec-project onboard|sync` modes;
  mirrors were derived from canonicals and all goldens regenerated.
- `install.sh` and `skills/autospec-project/install.sh` — verify/declare the
  installed typed command surface.
- `tests/autospec/managed-project-workflows.bats` and
  `tests/install/project-board-install.bats` — added executable ordering,
  provenance, shell-data, failure, and install contracts.

## Self-review

- `spawned-from` is opt-in and requires exactly one explicit repository; an
  ordinary adopted repository receives `contains` and no creation provenance.
- The control plane obtains `created` versus `adopted` before registration and
  performs the required `gh repo view <slug> --json url,defaultBranchRef`
  read-back before invoking Autospec.
- Source-spec/run values are passed as quoted arguments. No `eval` or generated
  shell program handles repository, owner, allowlist, workspace, or provenance
  input.
- Registration failure is warning-only after repository verification and push;
  it cannot re-enter repository creation or remove the remote.
- Owner onboarding remains fail-closed without one or more explicit literal or
  prefix allowlist entries.

## Concerns

- Task 6's stated file list omitted the Rust CLI module, but the required
  created-versus-adopted `spawned-from` distinction could not be represented by
  Task 4's prior options. The implementation adds one narrow, tested
  `--spawned-from` option rather than encoding provenance in shell-only prose.
- Live GitHub repository/Project mutations were not exercised; fake `gh` and
  real local bare repositories cover ordering, adoption, creation, and
  non-rollback behavior.
- Per instruction, no full workspace suite or repository-wide formatter was
  run. Focused Rust, Bats, syntax, install, trio, and golden gates are green.

## Fix round 1

Status: DONE_WITH_CONCERNS

### Commit

- `2af8919d` — `fix: preserve repository onboarding before projection`

### Corrected behavior

- Repository records, additive `contains` relationships, admission-gated
  `spawned-from` evidence, and idempotent `repository:register` projections are
  journaled before any GitHub Project resolution or retry.
- Repository record event keys use the distinct `repository:record` namespace,
  so a durable record and its projection cannot suppress one another. Repeated
  onboarding leaves one projection event per admitted repository.
- Retryable and ambiguous GitHub transport failures return the machine-readable
  `journaled_projection_pending` outcome with the real pending count and error
  summary. Definitive transport, remote-response validation, configuration,
  local storage, and unsupported outcome failures remain non-zero errors.
- `spawned-from` is written only when the normalized explicit repository is
  present in the onboarding report. Out-of-bound candidates receive neither
  that edge nor a repository projection.
- `autospec project sync` acknowledges preserved repository projections only
  after managed Project resolution and issue-projection retry succeed.
- Control-plane bootstrap accepts explicit `--source-spec IDENTITY`; that value
  takes precedence over `AUTOSPEC_RUN_ID` and the generic bootstrap identity.
  Values remain individually quoted shell arguments and are never evaluated.
- Control-plane bootstrap continues only for `reconciled` and
  `journaled_projection_pending`; it propagates every non-zero or unsupported
  result without rolling back or recreating the verified repository.
- The standalone `autospec-project` installer now invokes the established
  runtime installer when the configured binary lacks `autospec project`, then
  verifies the returned runtime path. It fails before claiming installation
  success when that surface remains unavailable.

### TDD and verification evidence

- RED: `cargo test -p autospec-cli --test managed_project onboard_cli_` failed
  to compile because `run_with_transport` returned no typed value; the new
  real-journal tests therefore could not observe a pending outcome.
- RED: focused control-plane Bats showed explicit `--source-spec` was rejected
  and exit `9` registration failures were swallowed as warning-only success.
- GREEN: `cargo test -p autospec-cli --test managed_project onboard_ --no-fail-fast`
  → `16 passed`, `0 failed`, including real `events.jsonl` ordering,
  idempotency, hard-versus-pending outcomes, admission gating, and later sync.
- GREEN: `bats tests/autospec/managed-project-workflows.bats`
  → `11 passed`, `0 failed`.
- GREEN: `bats tests/install/project-board-install.bats`
  → `10 passed`, `0 failed`.
- GREEN: `AUTOSPEC_TEST_BIN="$PWD/target/debug/autospec" bats
  tests/autospec/managed-project-onboard.bats` → `2 passed`, `0 failed`.
- GREEN: `cargo check -p autospec-cli` and `cargo clippy -p autospec-cli --bin
  autospec --no-deps` completed successfully; only pre-existing dead-code and
  unnecessary-cast warnings outside Task 6 were emitted.
- GREEN: `bash -n scripts/autospec-control-plane.sh`, `sh -n
  skills/autospec-project/install.sh`, targeted `rustfmt --check` with child
  traversal disabled, all four trio `--check` runs, idempotent golden
  regeneration, and `git diff --check` completed successfully.

### Remaining concerns

- Live GitHub repository/Project mutations remain untested; scripted transport
  and real local bare repositories cover ordering, durable recovery,
  provenance, and no-recreate behavior.
- A redundant broad `managed_project` integration-target run was stopped after
  it traversed unrelated autonomous Tier 2 tests and surfaced their existing
  failures. Per the scoped instruction, no full suite was pursued; all Task 6
  focused tests and static checks are green.

## Fix round 2

Status: DONE_WITH_CONCERNS

### Commit

- `e0e44876` — `fix: reserve pending outcomes for transient GitHub failures`

### Corrected behavior

- `GithubFailure::LocalExecution` now distinguishes a missing or broken local
  `gh` executable from retryable remote transport failure. It is an integrity
  block for accountability and a hard managed-project error.
- Read-only GitHub commands no longer classify every non-zero exit as
  retryable. Authentication, scope, HTTP 400/401/403/404, invalid response, and
  other non-transient failures are definitive. Recognized network, timeout,
  429/502/503/504, and rate-limit failures remain retryable; mutating calls
  retain ambiguous-response protection.
- Real binary-boundary tests run `autospec project onboard` through `GhCli`
  with a missing executable, a fake HTTP 403/auth executable, and a transient
  HTTP 503 executable. The first two exit non-zero with no pending summary; the
  transient case returns `journaled_projection_pending` with count `2`.
- The standalone project installer validates `project onboard --help`,
  `project sync --help`, and the literal `--spawned-from` capability for both
  an existing candidate and the runtime returned by the established installer.
  A generic successful `project --help` surface is rejected.
- The control-plane behavior test now requires the exact typed-pending warning
  `WARNING: managed Project repository registration journaled; projection
  remains pending (count=2)` twice—once for each companion repository.

### TDD and verification evidence

- RED: real GhCli tests showed missing executable and HTTP 403/auth read
  failures exited `0` with the typed pending outcome.
- RED: installer behavior tests showed an exact Task 6 runtime fixture was
  rejected while a generic `project --help` fixture was incorrectly accepted.
- GREEN: `cargo test -p autospec-cli --test managed_project gh_cli_
  --no-fail-fast` → `3 passed`, `0 failed`.
- GREEN: `cargo test -p autospec-cli --test managed_project onboard_
  --no-fail-fast` → `16 passed`, `0 failed`.
- GREEN: `bats tests/autospec/managed-project-workflows.bats`
  → `11 passed`, `0 failed`.
- GREEN: `bats tests/install/project-board-install.bats`
  → `11 passed`, `0 failed`.
- GREEN: targeted accountability retryability test, `cargo check -p
  autospec-cli`, `cargo clippy -p autospec-cli --bin autospec --no-deps`,
  installer syntax, targeted `rustfmt --check`, and `git diff --check` all
  completed successfully. Clippy emitted only the previously documented
  warnings outside Task 6.

### Remaining concern

- Live GitHub API error payloads remain untested; real process boundaries and
  representative stderr fixtures cover local execution, auth/scope, and
  transient transport classification without network mutation.
