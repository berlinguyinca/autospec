# Task 3 Report: Verified GitHub Project Upsert and Item Reconciliation

Status: DONE_WITH_CONCERNS

## Commit

- `b99d2479` — `feat: upsert managed GitHub Projects`
- Review fix — `fix: make managed Project retries fail closed`
- Review fix round 2 — `fix: keep created Projects provisional until verified`

## Red evidence

- `cargo test -p autospec-cli --test managed_project github_ -- --nocapture` → exit `101`;
  compilation failed on the missing lifecycle functions and Project command variants.
- `cargo test -p autospec-cli --test autonomous_accountability_github contracts -- --nocapture`
  → exit `101`; compilation failed on `ListProjects`, `CreateProject`, `ListProjectItems`,
  owner-explicit `AddToProject`, and the missing command renderer.
- The focused transport contract then failed on `project edit --readme-file -`; local `gh
  project edit --help` proved the supported contract is `--readme <string>`.
- The owner-conflict regression failed because a marker for the exact product but a different
  owner could fall through to creation instead of failing before mutation.
- The bounded-discovery transport regression failed before the explicit `--limit 500` and
  fail-closed truncation checks were added.
- Review round 1 added nine crash-recovery and malformed-response regressions; the focused
  `github_` run observed `10 passed`, `9 failed` before production changes. Failures proved
  that created identity was not durable before marker editing, pending creation could repeat,
  malformed README/item shapes were accepted or skipped, human README suffix bytes were
  trimmed, and numeric issue aliases were not canonicalized.
- Review round 2 first observed the new provisional-safety regression fail because an
  interrupted create exposed `Some("PVT_7")` as the final binding before marker verification.
  That state could authorize a later item-list/add transport call.

## Green evidence

- `cargo test -p autospec-cli --test managed_project github_ -- --nocapture` → exit `0`;
  `12 passed`, `0 failed`.
- `cargo test -p autospec-cli --test autonomous_accountability_github -- --nocapture` → exit
  `0`; `33 passed`, `0 failed`.
- `cargo test -p autospec-cli --test managed_project store_ -- --nocapture` → exit `0`;
  `21 passed`, `0 failed`.
- `cargo check -p autospec-cli --all-targets` → exit `0`; only pre-existing warnings.
- `cargo clippy -p autospec-cli --test managed_project --test
  autonomous_accountability_github` → exit `0`; only pre-existing warnings in path-included
  modules.
- Direct `rustfmt --check` over every touched Rust file and `git diff --check` → exit `0`.
- Review round 1: `cargo test -p autospec-cli --test managed_project github_ -- --nocapture`
  → exit `0`; `19 passed`, `0 failed`.
- Review round 1: `cargo test -p autospec-cli --test managed_project store_ -- --nocapture`
  → exit `0`; `21 passed`, `0 failed`.
- Review round 1: `cargo test -p autospec-cli --test autonomous_accountability_github --
  --nocapture` → exit `0`; `33 passed`, `0 failed`.
- Review round 2: `cargo test -p autospec-cli --test managed_project
  github_provisional_creation_cannot_authorize_item_mutation -- --nocapture` → exit `0`; the
  reopened provisional journal exposes no final binding and reconciliation makes zero calls.
- Review round 2: the focused `github_` and `store_` filters pass with `20` and `21` tests,
  respectively.

## Files changed

- `crates/autospec-cli/src/commands/managed_project/github.rs` — implements exact marker
  parsing, verified resolve/create/adopt, owner and truncation fail-closed checks, preserved
  human README content, normalized item membership, and pending-projection retries.
- `crates/autospec-cli/src/commands/managed_project/github/parse.rs` — isolates strict Project,
  README, marker, item, and canonical issue-URL parsing while retaining the 500-object
  fail-closed boundary.
- `crates/autospec-cli/src/commands/managed_project.rs` — exports the lifecycle surface and
  adds distinct provisional identity and final binding transitions with conflict checks.
- `crates/autospec-cli/src/commands/managed_project/store.rs` — replays the new
  `project-created` provisional event and promotes it only through the later `project-bound`
  event while preserving the existing serialized writer/checkpoint contract.
- `crates/autospec-cli/src/commands/autonomous/accountability/github/transport.rs` — adds
  owner-explicit Project list/view/create/edit/item commands and removes repository-derived
  ownership from `AddToProject`.
- `crates/autospec-cli/src/commands/autonomous/accountability/github.rs` — resolves the already
  validated repository owner before crossing the transport boundary.
- `crates/autospec-cli/tests/managed_project.rs` — scripted lifecycle coverage for creation,
  exact adoption, title-only refusal, ambiguity, owner conflict, truncation, idempotency,
  failure journaling, retry acknowledgment, create/edit crash recovery, malformed GitHub
  responses, byte-exact README preservation, known non-issue filtering, and canonical URLs.
- `crates/autospec-cli/tests/autonomous_accountability_github/contracts.rs` — exact argv/stdin
  contracts for the Project transport.

## Concerns

- `cargo fmt --all -- --check` remains non-green on pre-existing unrelated formatting drift in
  `autospec-core` tests; every Task 3 file passes direct rustfmt checking.
- The requested full workspace suite was intentionally not run; the task leader explicitly
  requested only scoped focused validation.
- Project discovery and item reads fail closed when 500 returned objects reach the configured
  transport limit. This avoids unsafe title adoption or duplicate item writes but requires a
  future paginated transport if products legitimately exceed that bound.
