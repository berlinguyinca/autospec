# AutoSpec Resilience/Memory/Learning Implementation Report

- **Date:** 2026-09-13
- **Source spec:** [`docs/specs/2026-09-13-autospec-artificium-resilience-memory-learning-spec.md`](../specs/2026-09-13-autospec-artificium-resilience-memory-learning-spec.md)
- **Primary owner:** `berlinguyinca/autospec`
- **Execution root:** `/home/wohlgemuth/IdeaProjects` (`AUTOSPEC_WORKSPACE_ROOT`)

## Workspace

- **root:** `/home/wohlgemuth/IdeaProjects`
- **repositories discovered (21):** `autospec`, `autospec-baselines`,
  `autospec-constitution`, `autospec-db`, `autospec-design`, `autospec-dispatcher`,
  `autospec-gui`, `autospec-inferweave` (archived), `autospec-node`,
  `autospec-orchestrator`, `autospec-ui-pilot`, and 11 generated `autospec-e2e-*`
  (handoff + listener) repositories.
- **repositories modified:** `autospec` (control plane — primary owner).
- **repositories classified reference/test-only:** `autospec-baselines`,
  `autospec-constitution`, `autospec-design`, `autospec-ui-pilot`, all
  `autospec-e2e-*` (generated fixtures/evidence), and the archived
  `autospec-inferweave`. `autospec-node` is a reference/inference-node repo.
- **dirty checkouts preserved (NOT reset, stashed, or overwritten):**
  - `autospec` — on `fix/quarantine-recheck-escape-hatch` with 54 modified files
    and untracked `tests/lint-deferral-refs.bats`,
    `scripts/lint-deferral-refs.sh`, `crates/.../harness_model_routing.rs`, `.omo/`.
  - `autospec-inferweave` — on `feat/seat-capacity-routing-observability` (clean;
    archived repo, not touched).

## Architecture

- **control-plane changes (`autospec`):** new `autospec_core::resilience` module
  (pure) holding the five feature contracts + identities + event names; a new
  `autospec resilience` CLI command; four versioned JSON Schemas; events contract
  doc; ADR 0002; this report. See ADR
  [`0002-resilience-runtime-placement.md`](../decisions/0002-resilience-runtime-placement.md).
- **dispatch-plane changes:** none in this slice. `autospec-dispatcher` remains
  the owner of dispatch judgement; it is a follow-up consumer of these contracts.
- **execution-plane changes:** none in this slice. `autospec-orchestrator` remains
  the owner of execution/session/lease mechanics; it consumes
  `resilience::work_protocol` and `resilience::context_guardian` as follow-up.
- **telemetry changes:** the `resilience::EVENTS` table and
  `docs/contracts/autospec-resilience-events-v1.md` define additive event names
  that `autospec-db` can mirror. Telemetry remains optional; never a correctness
  source.
- **UI changes:** none in this slice. `autospec-gui` read projections are follow-up.
- **memory-provider changes:** `resilience::memory_map::MemoryProvider` trait
  defines the narrow contract over the existing MemPalace integration
  (`skills/autospec-shared/scripts/mempalace-*.sh`). No new memory backend.

## Features

### Context Guardian
- **status:** implemented (pure) + tested.
- **proof:** `resilience::context_guardian` — threshold bands (soft 60 / warning
  75 / required 85), capability hierarchy exact→estimated→conservative→unknown,
  versioned checkpoint schema, bounded size, secret rejection, idempotent
  validation, phase ordering for crash distinction, resume plan, deterministic
  `may_begin_substantial_phase` gate. Tests: `resilience::context_guardian::tests`,
  plus `resilience_e2e::secret_in_checkpoint_is_rejected_end_to_end` and
  `unknown_context_usage_never_deadlocks`.

### Dynamic Memory Map
- **status:** implemented (pure) + tested.
- **proof:** `resilience::memory_map` — `MemoryProvider` trait, bounded
  `generate_map` that drops rather than truncates, explicit degraded mode with no
  fabrication, provenance/confidence, conflict resolution preferring recent
  validated memory and never a superseded entry. Tests in
  `resilience::memory_map::tests` and `resilience_e2e::unavailable_memory_degrades_without_fabrication`.

### Repository Attention Streams
- **status:** implemented (pure) + tested.
- **proof:** `resilience::attention_stream` — durable stream schema, chunk output
  contract, atomic cursor advancement, source-digest mutation detection,
  `resume_verdict` requiring reconciliation on material change, durable cancel.
  Tests in `resilience::attention_stream::tests` and
  `resilience_e2e::source_mutation_during_attention_stream_requires_reconciliation`.

### Durable Agent Work Protocol
- **status:** implemented (pure) + tested.
- **proof:** `resilience::work_protocol` — distinct `WorkId`/`AttemptId`/
  `ClaimId`/`SessionId`/`CheckpointId`/`ReceiptId`/`IdempotencyKey`/`ExecutionId`/
  `StreamId`; canonical happy-path + non-happy state machine; receipts with
  idempotency; atomic lease acquisition; heartbeat renewal; lease expiry;
  fencing generation preventing stale-worker finalization; deterministic
  recovery (block/resume/retry); terminal work never re-run. Tests in
  `resilience::work_protocol::tests` and `resilience_e2e::*`.

### Verified Engineering Learning
- **status:** implemented (pure) + tested.
- **proof:** `resilience::learning` — candidate schema, promotion gate requiring
  validation + review + no unresolved contradiction + confidence; rejection of
  secret-bearing lessons; supersession; `touches_immutable_policy`/`is_unsafe_lesson`
  guard so role/safety/merge policy can never be weakened by a lesson. Tests in
  `resilience::learning::tests`.

## Database/schema migrations

- No database migration in this slice (no schema change to any DB). `autospec-db`
  remains optional.
- Added versioned JSON Schemas (control-plane contract, not DB):
  - `schemas/autospec-context-checkpoint.schema.json`
  - `schemas/autospec-memory-map.schema.json`
  - `schemas/autospec-attention-stream.schema.json`
  - `schemas/autospec-work-receipt.schema.json`
  - `schemas/autospec-lesson-candidate.schema.json`

## Tests and validation

- `cargo build --workspace` — passes (only pre-existing `PHASE4_ADAPTER_FILES`
  dead-code warning).
- `cargo test -p autospec-core --lib resilience` — **35 passed, 0 failed**.
- `cargo test -p autospec-core --test resilience_e2e` — **8 passed, 0 failed**.
- `cargo test -p autospec-core --lib` — **424 passed, 0 failed**.
- `cargo test -p autospec-cli --test cli_commands cli_commands_help_lists_required_commands` — passes after adding `resilience` to the help snapshot.
- CLI smoke: `autospec resilience doctor`, `checkpoint-verdict`, `transition-check`,
  `lesson-verdict`, `events` all produce correct output.
- PR #4672 CI: `file-size-ratchet` passes (after splitting `work_protocol.rs` to stay
  under the 600-line cap); `main-builds`, `freebsd-test`, `macos-test`, `audit`,
  GitGuardian, security/stack-guard/ux-ui/python workstreams all pass.

## End-to-end scenario

`crates/autospec-core/tests/resilience_e2e.rs::end_to_end_dispatch_to_promotion_to_retrieval`
proves, with no harness/model/telemetry:

```text
dispatch (CREATED -> ASSIGNED)
-> execution (acquire lease)
-> context threshold reached (Required)
-> durable checkpoint generated + validated
-> resume allowed
-> implementation completion (DELIVERED -> CLAIMED -> RUNNING -> finalize -> COMPLETED)
-> deterministic validation (VALIDATED)
-> independent review (REVIEWED)
-> lesson candidate -> validated promotion (PROMOTED)
-> later memory-map retrieval surfaces the promoted lesson
```

Additional scenario tests: secret rejection, unavailable memory provider,
stale-worker fencing after reassignment, duplicate delivery idempotence, crash
recovery (block/resume/retry), unknown context usage, and attention-stream source
mutation requiring reconciliation.

## Pull requests / branches

- Branch: `feat/resilience-runtime-memory-learning` on `berlinguyinca/autospec`,
  opened as a PR against `main` (control-plane contracts + schemas + docs + tests).
- The dirty local branch `fix/quarantine-recheck-escape-hatch` was left untouched.

## Pre-existing failures (separate from this work)

Verified against clean `origin/main` in a detached worktree, the following fail
independently of this work:
- `executor_bridge` `codex_sandbox` interrupted-cleanup test and `pull_mutation`
  claim-takeover test (flaky, fail on clean main too).
- `convert_preflight` `a_missing_tool_in_plan_mode_warns_without_refusing`.

### PR #4672 build-test failures (all three reproduced on clean `origin/main`)

The full-catalog build-test reports `total=164 passed=161 failed=3`. All three
fail identically on clean `origin/main` and none reference this work:
- `check_dogfood_detectors` — `qa-brute-force-sweep.sh` reports 54 findings vs 32
  expected (allowlist drift across many pre-existing files: `claim.rs`, `cleanup.rs`,
  `convert.rs`, `construction_sites.rs`, `dispatch_pipeline.rs`, `ci_conclusions.rs`,
  `prose_closure.rs`, `wire_fixture.rs`, etc.). Zero `resilience/` files are flagged.
- `check_install_tests` — `tests/install/*.sh` hang on network (local repro: exit 124).
- `check_autonomous_phase2_suite` — `tests/autonomous/test_accessibility_workstream.bats`
  `not ok 5` fails on missing `.github/workflows/accessibility-workstream.yml`, which does
  not exist on `main`.

### TeamCity gates
- `file-size-ratchet`: **passes** after the split commit.
- `architecture-fitness`: **pre-existing failure on `main`** — `rust_core_cli_direction`
  reports observed=73 on clean `origin/main`; identical on this branch.

On the preserved dirty branch (not caused by this work):
- `validation_parity::direct_plans_match_the_frozen_catalog` — the branch added a
  `check_lint_deferral_refs` validation check (154 vs frozen 153) but the frozen
  `catalog-v1.json` was not regenerated.
- `validation_runner::every_unbaselined_bats_suite_...` — untracked
  `tests/lint-deferral-refs.bats` is not registered by any validate check.
- `autonomous_conductor_commands::foreground_rejects_foreign_released_predecessor_heartbeat_before_acquire`
  — in the branch's in-flight claim/heartbeat work; unrelated to this slice.

## Remaining risks

- The `autospec` checkout is dirty on `fix/quarantine-recheck-escape-hatch`; the
  PR branch was created from that state. Any merge must reconcile with the
  in-flight branch work.
- `autospec-orchestrator` origin is `InferWeave/autospec-orchestrator` (a
  redirect); noted but not blocking.
- Pure control-plane contracts are implemented; cross-plane wiring (orchestrator
  checkpoint handshake + lease mechanics, dispatcher judgement, `autospec-db`
  event projection, `autospec-gui` read views) is follow-up work that consumes
  these contracts.

## Follow-up issues

1. Orchestrator: implement the checkpoint handshake + durable checkpoint store
   using `resilience::context_guardian`; wire `resilience::work_protocol` lease
   mechanics into `orchestrator-worker`.
2. Dispatcher: use `resilience::work_protocol` state transitions + receipts for
   attempt selection and idempotence.
3. `autospec-db`: add additive projections for `resilience::EVENTS` + Grafana
   panels.
4. `autospec-gui`: add read views for active work, checkpoints, leases, attention
   streams, lesson candidates.
5. Memory: add a MemPalace-backed `MemoryProvider` implementation (shelling to
   `mempalace search` / `wake-up`) and wire `generate_map` into session start.
