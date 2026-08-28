# Task 4 Report: Bounded Existing-Repository Onboarding

Status: DONE_WITH_CONCERNS

## Commit

- `6351305c` — `feat: onboard existing repository relationships`

## Red evidence

- `cargo test -p autospec-cli --test managed_project onboard_ -- --nocapture` initially
  failed with unresolved imports for `onboard_repositories`, `OnboardingOptions`, and
  `active_dependency_graph`.
- The first scanner run failed on an npm `github:` dependency, then a Go
  `github.com/owner/repo` replacement, proving the canonicalizer did not yet cover those
  supported manifest forms.
- `onboard_cli_dry_run_emits_stable_sorted_json` initially failed with
  `unknown autospec command: project`.
- Both Bats cases initially failed because the `project` command was absent.
- The dry-run non-mutation regression failed while the command created
  `.autospec/state`; the final implementation uses an isolated temporary store and removes it.
- The multiline Cargo workspace regression failed with a missing workspace repository before
  the bounded workspace-member parser was added.

## Green evidence

- Direct `rustfmt --check` with child traversal disabled over all four Task 4 Rust files and
  `git diff --check` → exit `0`.
- `cargo test -p autospec-cli --test managed_project onboard_ -- --nocapture` → exit `0`;
  `3 passed`, `0 failed`.
- `cargo test -p autospec-cli --test managed_project store_ -- --nocapture` → exit `0`;
  `21 passed`, `0 failed`.
- `cargo test -p autospec-cli --test managed_project github_ -- --nocapture` → exit `0`;
  `21 passed`, `0 failed`.
- `bats tests/autospec/managed-project-onboard.bats` → exit `0`; `2 passed`, `0 failed`.
- `cargo check -p autospec-cli --bin autospec` → exit `0`; only pre-existing dead-code
  warnings outside Task 4.
- `cargo clippy -p autospec-cli --test managed_project` → exit `0`; Task 4's initial
  too-many-arguments warning was removed. Remaining warnings are in pre-existing path-included
  modules.

## Files changed

- `crates/autospec-cli/src/commands/managed_project/onboard.rs` — canonical GitHub repository
  admission, deterministic bounded scanners, stable report model, and active dependency graph.
- `crates/autospec-cli/src/commands/managed_project.rs` — `project resolve|sync|onboard`
  command parsing, managed-policy loading, Task 3 reconciliation reuse, stable JSON, and
  non-mutating dry-run state isolation.
- `crates/autospec-cli/src/main.rs` — routes the `project` command without changing other CLI
  dispatch.
- `crates/autospec-cli/tests/managed_project.rs` — TDD coverage for boundaries, scanner
  evidence, discovery limits, repeatability, proposal isolation, workspace members, and CLI
  stability.
- `tests/autospec/managed-project-onboard.bats` — binary-level dry-run and boundary contracts.

## Self-review

- Confirmed scanner execution is limited to file reads plus `git remote get-url`; no project
  code, build scripts, or package managers are executed.
- Confirmed every queued local or discovered repository is normalized and checked against both
  configured owner and allowlist.
- Confirmed concrete submodule, manifest, workspace, fleet, issue, source-spec, and tracker
  evidence produces active typed edges; name-only references produce proposed edges.
- Confirmed `active_dependency_graph` returns only active `depends-on` and `blocks` edges.
- Confirmed BTree-backed repository and edge accumulation makes report output stable and
  repeated non-dry-run onboarding leaves repository and relationship sets unchanged.
- Confirmed dry-run does not create repository-local managed state or call GitHub.

## Concerns

- Live `gh project` resolve/sync mutations were not exercised; Task 3's scripted GitHub
  lifecycle suite remains green and Task 4 delegates to that verified service unchanged.
- The focused integration test path includes many pre-existing dead-code and clippy warnings
  because it imports `commands/mod.rs`; no new Task 4 warning remains.
- Per parent instruction, no full workspace suite or repository-wide formatter was run.
