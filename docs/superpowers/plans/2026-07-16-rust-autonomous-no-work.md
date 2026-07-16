# Rust Autonomous No-Work and Ideation Implementation Plan

> **Superseded after Task 1:** The pure no-work foundation is complete. The
> original manual `record|status` adapter cannot close #1872 because it would
> not run a full Rust waterfall or produce a local backlog. Continue with
> [`rust-autonomous-waterfall.md`](2026-07-16-rust-autonomous-waterfall.md).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide a typed Rust no-work record and status command that requests bounded planning-only ideation after two consecutive verified full dry waterfalls.

**Architecture:** A pure core model validates closed tier results and derives consecutive dry-pass decisions. The CLI parses a strict record command, atomically persists one repo-scoped artifact, and renders a read-only status projection without any shell, GitHub, or work-dispatch authority.

**Tech Stack:** Rust 2021 standard library, existing core JSON parser, CLI `atomic_write`, Cargo tests; no new dependencies.

## Global Constraints

- Ordered tiers: `tier1`, `tier1_5`, `tier2`, `tier3`, `tier4`; exactly one `--tier` value per tier.
- Exact dry reasons: `no_proposals_generated`, `deduplicated`, `verification_rejected`, `roi_filtered`, `already_implemented`.
- `not_run`, `failed`, and `produced` are never dry; only all-five dry is a full dry pass.
- Threshold is the typed constant `2`, with no new environment or shell configuration.
- `--pass` is a positive idempotent integer; stale or conflicting duplicate pass data fails closed.
- Ideation is `planning_only`, candidate limit `5`, and remote mutation `none`.
- Never run shell, `omx`, `gh`, agent/model, queue/claim/label logic, or issue creation in this command.
- Do not edit legacy waterfall scripts or skill trios.

---

### Task 1: Add the pure no-work policy and JSON codec

**Files:** Create `crates/autospec-core/src/autonomous/no_work.rs`; modify `crates/autospec-core/src/lib.rs`; create `crates/autospec-core/tests/autonomous_no_work.rs`.

**Interfaces:** `NoWorkTier`, `DryReason`, `TierOutcome`, `NoWorkObservation`, `NoWorkState`, and `NoWorkDecision`. `NoWorkState::record(previous, observation) -> Result<NoWorkState, String>` owns pass ordering/idempotency and returns `IdleRescan` or `RequestIdeation`.

- [ ] **Step 1: Write failing core tests**

```rust
#[test]
fn second_consecutive_complete_dry_pass_requests_bounded_ideation() {
    let first = NoWorkState::record(None, complete_dry(1)).expect("first pass");
    let second = NoWorkState::record(Some(&first), complete_dry(2)).expect("second pass");
    assert_eq!(second.decision(), NoWorkDecision::RequestIdeation);
    assert_eq!(second.candidate_limit(), 5);
}

#[test]
fn not_run_failed_and_produced_tiers_cannot_increment_a_dry_pass() {
    for observation in [with_not_run(1), with_failed(1), with_produced(1)] {
        assert_eq!(NoWorkState::record(None, observation).unwrap().consecutive_dry_passes(), 0);
    }
}
```

Also require every exact dry reason, missing/duplicate tier rejection, duplicate-pass idempotency, conflicting duplicate rejection, stale-pass rejection, JSON round-trip, and exact six-question request projection.

- [ ] **Step 2: Verify the core test fails**

Run: `cargo test -p autospec-core --test autonomous_no_work --quiet`

Expected: FAIL because the no-work module and public types do not exist.

- [ ] **Step 3: Implement the pure closed model**

```rust
pub const IDEATION_DRY_PASS_THRESHOLD: u64 = 2;
pub const IDEATION_CANDIDATE_LIMIT: u64 = 5;

pub enum TierOutcome { Produced { count: u64 }, Dry { reason: DryReason }, NotRun { reason: String }, Failed { reason: String } }

pub fn record(previous: Option<&NoWorkState>, observation: NoWorkObservation) -> Result<NoWorkState, String>;
```

Validate the ordered complete tier set, positive produced counts/pass ID, nonempty not-run/failed reason, and identity consistency. Duplicate matching pass returns the previous state unchanged. A newer full dry pass increments the counter; every other newer pass resets it. Encode/decode only the schema-1 closed JSON shape using existing `JsonParser` utilities.

- [ ] **Step 4: Verify the core policy**

Run: `cargo test -p autospec-core --test autonomous_no_work --quiet`

Expected: PASS for the full pure decision and codec matrix.

- [ ] **Step 5: Commit the core policy**

Stage the new module, export, and test. Commit `feat: model Rust autonomous no-work decisions` with Lore trailers.

### Task 2: Add the Rust-only record and status adapter

**Files:** Create `crates/autospec-cli/src/commands/autonomous/no_work.rs`; modify `crates/autospec-cli/src/commands/autonomous.rs`; modify `crates/autospec-cli/tests/autonomous_conductor_commands.rs`.

**Interfaces:**

```text
autospec autonomous no-work record --repo OWNER/REPO --pass N \
  --tier tier1=DRY_REASON --tier tier1_5=DRY_REASON \
  --tier tier2=DRY_REASON --tier tier3=DRY_REASON --tier tier4=DRY_REASON [--json]
autospec autonomous no-work status --repo OWNER/REPO [--json]
```

The record parser also accepts `tier=not_run:REASON`, `tier=failed:REASON`, and `tier=produced:COUNT` exactly once per tier.

- [ ] **Step 1: Write failing CLI integration tests**

```rust
#[test]
fn second_dry_record_writes_a_planning_only_ideation_request() {
    fixture.record_complete_dry(1).expect("first record");
    let output = fixture.record_complete_dry(2).expect("second record");
    assert!(output.status.success());
    let artifact = fixture.read_no_work_artifact();
    assert!(artifact.contains("\"decision\":\"ideation_backlog_refresh_required\""));
    assert!(artifact.contains("\"candidate_limit\":5"));
    assert!(artifact.contains("\"remote_mutation\":\"none\""));
}
```

Also prove `status --json` reads the artifact, malformed repo/argument fails with exit 2 before an artifact write, a conflicting same pass does not replace the artifact, a foreign/malformed existing artifact fails closed, and no fake GitHub call occurs.

- [ ] **Step 2: Verify the integration test fails**

Run: `cargo test -p autospec-cli --test autonomous_conductor_commands second_dry_record_writes_a_planning_only_ideation_request --quiet`

Expected: FAIL because `autonomous no-work` is unknown.

- [ ] **Step 3: Implement strict adapter and atomic persistence**

Dispatch `no-work` before generic option parsing. Build `RunLayout`, validate `RepositoryScope`, read only `state_dir/why-no-work.json`, parse it through the core codec, and use `super::atomic_write` for replacement. `record` prints the schema-1 artifact in JSON mode; `status` returns the validated artifact without writes. Build the six fixed questions and score-field names from constants, never from user input.

- [ ] **Step 4: Verify CLI behavior**

Run: `cargo test -p autospec-cli --test autonomous_conductor_commands --quiet`

Expected: PASS; no-work records are atomic, idempotent, and local-only.

- [ ] **Step 5: Commit the adapter**

Stage command, dispatcher, and integration tests. Commit `feat: record Rust autonomous no-work evidence` with Lore trailers.

### Task 3: Document authority limits and validate

**Files:** Modify `docs/cli-reference.md`; create `docs/runbooks/autonomous-no-work.md`; update the design and this plan; extend `crates/autospec-cli/tests/autonomous_conductor_commands.rs`.

- [ ] **Step 1: Write a failing source-authority guard**

```rust
#[test]
fn rust_no_work_adapter_has_no_legacy_or_remote_mutation_authority() {
    let source = fs::read_to_string(workspace_root().join("crates/autospec-cli/src/commands/autonomous/no_work.rs")).unwrap();
    for forbidden in ["Command::new(\"sh\")", "Command::new(\"bash\")", "Command::new(\"gh\")", "omx", "autonomous-waterfall.sh", "autospec-loop.sh", "auto-implement", "issue create"] {
        assert!(!source.contains(forbidden), "forbidden authority: {forbidden}");
    }
}
```

- [ ] **Step 2: Verify the guard fails**

Run: `cargo test -p autospec-cli --test autonomous_conductor_commands rust_no_work_adapter_has_no_legacy_or_remote_mutation_authority --quiet`

Expected: FAIL because the named test is absent.

- [ ] **Step 3: Document the command and artifact**

Document record syntax, all exact dry reasons, pass semantics, artifact path, two-pass threshold, six question contract, candidate-limit five, planning-only disposition, and explicit non-authority for GitHub/work mutation. State that future typed waterfall adapters—not legacy shell loops—supply actual tier observations.

- [ ] **Step 4: Run full validation**

Run: `cargo fmt --all --check && cargo clippy --workspace -- -D warnings && cargo test --workspace --quiet && cargo run -q -p autospec-cli -- validate --fast`

Expected: every command exits 0.

- [ ] **Step 5: Commit documentation and guard**

Stage docs, plan/spec, and static guard. Commit `docs: define Rust autonomous no-work artifacts` with Lore trailers.

## Plan Self-Review

Task 1 owns pure policy and closed persistence. Task 2 is the only I/O adapter
and cannot invoke remote or execution authority. Task 3 documents the exact
safe boundary. The plan satisfies #1872's observable dry reasons and bounded
ideation request while truthfully leaving unported tiers `not_run`.
