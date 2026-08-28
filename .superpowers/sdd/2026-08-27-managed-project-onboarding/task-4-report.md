# Task 4 Report: Bounded Existing-Repository Onboarding

Status: DONE_WITH_CONCERNS

## Commits

- `6351305c` — `feat: onboard existing repository relationships`
- `06c8965a` — `fix: preserve bounded onboarding evidence`
- `d65f1719` — `fix: validate onboarding before side effects`
- `4c12501d` — `fix: constrain npm repository metadata to GitHub`

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
- Round 2 regressions demonstrated that malformed policy/CLI seeds reached writable-store and
  GitHub setup, read-only open returned a stale binding without journal replay, explicit boundary
  seeds consumed the expansion cap, and npm semver/alias/file/workspace protocols inflated the
  inaccessible count. Four RED runs covering five focused tests failed for those exact causes
  before production edits.
- The restored positive name-reference regression passed immediately, confirming that the
  existing proposal-state isolation was correct but previously lacked under-cap coverage.
- Round 3 positive and negative npm regressions both failed before production edits:
  `git+https://github.com/...` repository metadata was counted inaccessible, while GitLab
  `repository` metadata was emitted and also inflated the inaccessible count.

## Green evidence

- Direct `rustfmt` over the changed Task 4 Rust files with child traversal disabled plus
  `git diff --check` → exit `0`.
- `cargo test -p autospec-cli --test managed_project onboard_ --no-fail-fast` → exit `0`;
  `12 passed`, `0 failed`.
- `cargo test -p autospec-cli --test managed_project github_ --no-fail-fast` → exit `0`;
  `21 passed`, `0 failed`.
- `cargo test -p autospec-cli --test managed_project store_ --no-fail-fast` → exit `0`;
  `23 passed`, `0 failed`.
- `bats tests/autospec/managed-project-onboard.bats` → exit `0`; `2 passed`, `0 failed`.
- `cargo check -p autospec-cli` and `cargo clippy -p autospec-cli --bin autospec --no-deps`
  → exit `0`; warnings are pre-existing and outside Task 4.
- Managed-project production modules are all below 500 lines; the largest is `store.rs` at 465
  lines.

## Files changed

- `crates/autospec-cli/src/commands/managed_project/onboard.rs` — bounded typed discovery,
  endpoint-before-edge retention, orchestration, and active dependency graph.
- `crates/autospec-cli/src/commands/managed_project/onboard/*.rs` — structured Cargo, npm/pnpm,
  line-format, managed-issue, admission, and stable-report modules, including npm `git+https`
  and `git+ssh` GitHub canonicalization.
- `crates/autospec-cli/src/commands/managed_project/cli.rs` — focused
  `project resolve|sync|onboard` parsing and reporting; every policy/CLI seed is validated before
  writable state or GitHub, and dry-run suppresses GitHub plus persistence.
- `crates/autospec-cli/src/commands/managed_project/project.rs` — extracted project identity
  persistence without behavior changes.
- `crates/autospec-cli/src/commands/managed_project/store.rs` and
  `store/recovery.rs` — read-only journal replay/checkpoint validation without repair or writes,
  while writable opens retain truncated-tail repair.
- `crates/autospec-cli/src/commands/managed_project.rs` — bounded module wiring.
- `crates/autospec-cli/tests/managed_project.rs` — regressions for dry-run state preservation,
  endpoint admission, structured-field false positives, Cargo path/pnpm workspace discovery,
  typed failures, pre-side-effect seed validation, read-only canonical replay, explicit-vs-
  expansion bounds, npm protocol filtering, malformed seeds, and proposal isolation.
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
- The repository bound is applied only to scanner expansion before retaining new records or their
  edges; explicit policy, CLI, and workspace boundary seeds remain authoritative even above that
  cap. Local workspace outcomes distinguish admitted, out-of-bound, and inaccessible paths.
- Proposed name-only relationships remain `Proposed`; `active_dependency_graph` exposes only
  active dependency/blocking edges.
- Malformed explicit policy or CLI seeds fail closed before a writable store is opened or GitHub
  is called. The fake-transport regression verifies zero calls and absent state.
- Dry-run replays completed journal events into the newest canonical snapshot, validates the
  persisted checkpoint, ignores an incomplete tail in memory, and never truncates or rewrites the
  binding/journal. A nonempty binding without a valid journal fails closed.
- npm dependencies yield repository candidates only when the dependency value canonicalizes as a
  GitHub repository; semver, registry alias, file, and workspace protocols are ignored rather than
  counted inaccessible.
- npm `repository` strings/object URLs and dependency values share the same canonical GitHub
  predicate. GitLab and malformed metadata are ignored, while npm `git+https` and `git+ssh`
  GitHub URLs normalize to `owner/repo`.

## Concerns

- Live `gh project` resolve/sync mutations were not exercised; the adjacent scripted GitHub
  lifecycle tests remain green and onboarding delegates to that service unchanged.
- The focused integration test imports broad command modules, producing pre-existing dead-code
  warnings; no warning originates in the changed managed-project modules.
- Per parent instruction, no full workspace suite or repository-wide formatter was run.
