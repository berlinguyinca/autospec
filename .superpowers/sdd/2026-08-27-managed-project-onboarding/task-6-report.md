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
