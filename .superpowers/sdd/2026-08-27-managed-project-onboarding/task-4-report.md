# Task 4 Report: Bounded Existing-Repository Onboarding

Status: DONE_WITH_CONCERNS

## Commits

- `6351305c` — `feat: onboard existing repository relationships`
- `06c8965a` — `fix: preserve bounded onboarding evidence`

## Review-fix red evidence

- The six new/strengthened `onboard_` regressions initially produced `1 passed`, `5 failed`:
  dry-run read an empty temporary store, over-bound edges survived without admitted endpoints,
  whole-file token scanning retained unrelated URLs, Cargo path and pnpm workspace repositories
  were absent, workspace failures were not typed or counted, and malformed explicit seeds were
  silently ignored.
- The cap regression demonstrated that relationships were retained before the target repository
  passed `discovery_max_repos` admission.
- Structured-field fixtures demonstrated false positives from Cargo comments/package metadata,
  package scripts/homepage, and unrelated fleet metadata.

## Green evidence

- Direct `rustfmt` over the changed Task 4 Rust files with child traversal disabled plus
  `git diff --check` → exit `0`.
- `cargo test -p autospec-cli --test managed_project onboard_ --no-fail-fast` → exit `0`;
  `6 passed`, `0 failed`.
- `cargo test -p autospec-cli --test managed_project github_ --no-fail-fast` → exit `0`;
  `21 passed`, `0 failed`.
- `cargo test -p autospec-cli --test managed_project store_ --no-fail-fast` → exit `0`;
  `21 passed`, `0 failed`.
- `bats tests/autospec/managed-project-onboard.bats` → exit `0`; `2 passed`, `0 failed`.
- `cargo check -p autospec-cli` and `cargo clippy -p autospec-cli --bin autospec --no-deps`
  → exit `0`; warnings are pre-existing and outside Task 4.
- Managed-project production modules are all below 500 lines; the largest Task 4 module is
  `onboard.rs` at 446 lines.

## Files changed

- `crates/autospec-cli/src/commands/managed_project/onboard.rs` — bounded typed discovery,
  endpoint-before-edge retention, orchestration, and active dependency graph.
- `crates/autospec-cli/src/commands/managed_project/onboard/*.rs` — structured Cargo, npm/pnpm,
  line-format, managed-issue, admission, and stable-report modules.
- `crates/autospec-cli/src/commands/managed_project/cli.rs` — focused
  `project resolve|sync|onboard` parsing and reporting; dry-run opens the real persisted snapshot
  read-only and suppresses GitHub plus persistence.
- `crates/autospec-cli/src/commands/managed_project/project.rs` — extracted project identity
  persistence without behavior changes.
- `crates/autospec-cli/src/commands/managed_project/store.rs` and
  `store/recovery.rs` — read-only snapshot opening and extracted journal recovery.
- `crates/autospec-cli/src/commands/managed_project.rs` — bounded module wiring.
- `crates/autospec-cli/tests/managed_project.rs` — regressions for dry-run state preservation,
  endpoint admission, structured-field false positives, Cargo path/pnpm workspace discovery,
  typed failures, and malformed seeds.
- `tests/autospec/managed-project-onboard.bats` — binary-level dry-run and boundary contracts
  retained from the initial implementation.

## Self-review

- Discovery reads only supported data files and invokes only `git remote get-url` to identify a
  repository-local workspace; it never executes project code, scripts, or package managers.
- GitHub tokens are parsed only from dependency, repository, workspace, fleet-repository, and
  Autospec-managed relationship fields. Comments, scripts, homepage/description metadata, and
  unrelated fleet fields do not create relationships.
- Owner and allowlist admission occurs before queueing, repository retention, or edge retention.
  A newly retained active edge therefore has both endpoints in the admitted repository set.
- The repository bound is applied before retaining new records or their edges. Local workspace
  outcomes distinguish admitted, out-of-bound, and inaccessible paths, and the latter two are
  counted stably.
- Proposed name-only relationships remain `Proposed`; `active_dependency_graph` exposes only
  active dependency/blocking edges.
- Malformed explicit policy or CLI seeds fail closed before persistence. Dry-run evaluates the
  existing snapshot and reports existing repositories/pending projections without modifying the
  binding or journal and without calling GitHub.

## Concerns

- Live `gh project` resolve/sync mutations were not exercised; the adjacent scripted GitHub
  lifecycle tests remain green and onboarding delegates to that service unchanged.
- The focused integration test imports broad command modules, producing pre-existing dead-code
  warnings; no warning originates in the changed managed-project modules.
- Per parent instruction, no full workspace suite or repository-wide formatter was run.
