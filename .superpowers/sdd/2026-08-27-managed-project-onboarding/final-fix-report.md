# Final integration fix report

Date: 2026-08-28

## Outcome

All final review Critical and Important findings, plus the requested complexity Minor, are resolved in the managed-project onboarding branch.

1. Issue projections are normalized and journaled under an unresolved key before Project lookup, then durably promoted to the verified Project node identity. Project auth, scope, marker, creation, listing, and item-add failures therefore retain retryable local intent.
2. `project onboard` accepts repeatable `--issue-url` selections for existing open or closed issues, validates every issue against the managed owner/allowlist before state or GitHub access, reconciles every admitted selection, and reports selected/reconciled counts.
3. The issue scanner retains canonical issue/PR identities, canonicalizes its numeric source issue when the managed issue filename supplies one, preserves same-repository issue edges, and keeps `DependsOn` distinct from `Blocks`.
4. The read-only `project active-edges` projection feeds board readiness. `project-ship.sh` launches only repositories with open, ready work after typed `DependsOn`/`Blocks` overlay, including cross-repository dependencies.
5. Production state is product-global at `${AUTOSPEC_HOME:-$HOME/.autospec}/projects/<product-key>`. A valid legacy repository-local journal is copied under the global product lock only when no global state exists; the legacy source remains intact for recovery/audit.
6. Read-only loading validates the state root, `projects`, and product directory with `symlink_metadata`, rejecting symlinks, non-directories, foreign ownership, and group/world permissions before reading state files.
7. `onboard_repositories` is 30 LOC and `apply_event` is 26 LOC. Event handling and discovery phases are split into focused helpers. The broad managed-project dead-code/unused-import suppression was removed; test-only exports use narrow `cfg(test)` gates.

## TDD evidence

### Red

- Unresolved issue projection regression initially failed to compile because `journal_issue_projection` did not exist.
- Blocks/canonical target regression initially produced `DependsOn` and repository-only target identity.
- Same-repository issue dependency regression failed with no retained edge.
- Global store regression initially failed because `ManagedProjectStore::open_global` did not exist.
- Read-only ancestor regression admitted a public or symlinked state ancestor.
- Repeatable onboarding selections were initially rejected/unsupported.
- Board/fleet readiness had no managed active-edge projection or consumer.

### Green

- `cargo test -p autospec-cli --test managed_project github_` — 24 passed.
- `cargo test -p autospec-cli --test managed_project store_` — 25 passed.
- `cargo test -p autospec-cli --test managed_project onboard_` — 25 passed.
- `bats tests/autospec/managed-project-onboard.bats tests/autospec/project-board-deps.bats tests/autospec/project-board-resolve.bats tests/fleet/project-ship.bats` — 97 passed.
- `bats tests/gen-skill-goldens.bats` — 9 passed; skill/golden lock-step remains intact.
- `cargo check -p autospec-cli` — passed.
- `git diff --check` — passed.

## Safety and compatibility notes

- Legacy state import is copy-first and non-destructive. It replays and validates the legacy journal before copying, serializes import under the global product lock, and never overwrites an existing global binding or journal.
- Read-only commands prefer global state and fall back to a valid legacy repository-local state only when no global product directory exists.
- Issue URL and repository admission happen before mutable store open or GitHub Project calls.
- Active-edge overlay ignores malformed/non-issue endpoints and proposed edges; it never invents dependency identities.

## Remaining concerns

- Per dispatch instruction, the full workspace suite was not run. Verification was limited to the managed-project Rust filters, board/fleet Bats suites, the skill golden check, and `cargo check -p autospec-cli`.
- No live GitHub Project mutation was performed. Remote behavior is covered by the scripted transport and shell seams.
- `cargo check` still reports four pre-existing dead-code warnings in claim/heartbeat code and one pre-existing `autospec-core` warning; this change introduces no broad suppression for them.
